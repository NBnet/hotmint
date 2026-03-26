//! Hotmint EVM Node — production-grade EVM-compatible chain.
//!
//! Runs a single validator node with real P2P networking (litep2p).
//! Uses the same `--home` config layout as `cluster-node` plus an
//! additional `config/evm-genesis.json` for EVM-specific settings.
//!
//! Usage:
//!   hotmint-evm --home /path/to/node/home [--rpc-addr 127.0.0.1:8545]

use ruc::*;

use std::sync::Arc;

use clap::Parser;
use tokio::sync::RwLock;
use tracing::info;

use hotmint::config::{self, GenesisDoc, NodeConfig, NodeKey, PrivValidatorKey};
use hotmint::consensus::engine::{ConsensusEngine, EngineConfig};
use hotmint::consensus::pacemaker::PacemakerConfig;
use hotmint::consensus::state::ConsensusState;
use hotmint::consensus::store::BlockStore;
use hotmint::crypto::{Ed25519Signer, Ed25519Verifier};
use hotmint::network::service::{NetworkService, PeerMap};
use hotmint::prelude::*;
use hotmint::storage::block_store::VsdbBlockStore;
use hotmint::storage::consensus_state::PersistentConsensusState;

use hotmint_evm_execution::{EvmExecutor, SharedExecutor};
use hotmint_evm_rpc::{EvmRpcState, start_rpc_server};
use hotmint_evm_types::genesis::EvmGenesis;

/// Hotmint EVM Node — real P2P networking, full EVM execution.
#[derive(Parser)]
#[command(name = "hotmint-evm", about = "Hotmint EVM-compatible chain node")]
struct Cli {
    /// Path to hotmint home directory (contains config/, data/).
    #[arg(long)]
    home: String,

    /// Ethereum JSON-RPC listen address (host:port).
    /// Overrides the config file's RPC address for the eth_* endpoints.
    #[arg(long, default_value = "127.0.0.1:8545")]
    rpc_addr: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let home = std::path::Path::new(&cli.home);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    if let Err(e) = run(home, &cli.rpc_addr).await {
        eprintln!("Fatal: {e}");
        std::process::exit(1);
    }
}

async fn run(home: &std::path::Path, eth_rpc_addr: &str) -> Result<()> {
    let config_dir = home.join("config");
    let data_dir = home.join("data");

    // Load standard hotmint config files.
    let node_config =
        NodeConfig::load(&config_dir.join("config.toml")).c(d!("failed to load config.toml"))?;
    let priv_key = PrivValidatorKey::load(&config_dir.join("priv_validator_key.json"))
        .c(d!("failed to load priv_validator_key.json"))?;
    let signing_key = priv_key.to_signing_key()?;
    let node_key =
        NodeKey::load(&config_dir.join("node_key.json")).c(d!("failed to load node_key.json"))?;
    let litep2p_keypair = node_key.to_litep2p_keypair()?;

    let genesis =
        GenesisDoc::load(&config_dir.join("genesis.json")).c(d!("failed to load genesis.json"))?;
    let validator_set = genesis.to_validator_set()?;

    // Load EVM genesis — application-specific config.
    let evm_genesis_path = config_dir.join("evm-genesis.json");
    let evm_genesis = if evm_genesis_path.exists() {
        EvmGenesis::load(&evm_genesis_path).c(d!("failed to load evm-genesis.json"))?
    } else {
        info!("No evm-genesis.json found, using default dev genesis");
        default_dev_genesis()
    };

    // Identify this validator.
    let our_pk_hex = &priv_key.public_key;
    let our_gv = genesis
        .validators
        .iter()
        .find(|v| &v.public_key == our_pk_hex)
        .ok_or_else(|| eg!("this node's public key not found in genesis"))?;
    let our_vid = ValidatorId(our_gv.id);

    info!(
        validator_id = %our_vid,
        chain_id = evm_genesis.chain_id,
        accounts = evm_genesis.alloc.len(),
        gas_limit = evm_genesis.gas_limit,
        validators = validator_set.validator_count(),
        "=== Hotmint EVM Node ==="
    );

    // Storage.
    std::fs::create_dir_all(&data_dir).c(d!("create data dir"))?;
    vsdb::vsdb_set_base_dir(&data_dir).c(d!("set vsdb base dir"))?;

    let store: Arc<parking_lot::RwLock<Box<dyn BlockStore>>> =
        Arc::new(parking_lot::RwLock::new(Box::new(VsdbBlockStore::new())));

    // Restore consensus state.
    let pcs = PersistentConsensusState::new();
    let mut state =
        ConsensusState::with_chain_id(our_vid, validator_set.clone(), &genesis.chain_id);
    if let Some(view) = pcs.load_current_view() {
        state.current_view = view;
    }
    if let Some(qc) = pcs.load_locked_qc() {
        state.locked_qc = Some(qc);
    }
    if let Some(qc) = pcs.load_highest_qc() {
        state.highest_qc = Some(qc);
    }
    if let Some(h) = pcs.load_last_committed_height() {
        state.last_committed_height = h;
    }
    if let Some(epoch) = pcs.load_current_epoch() {
        state.validator_set = epoch.validator_set.clone();
        state.current_epoch = epoch;
    }

    // P2P Networking.
    let (peer_map, known_addresses) = if node_config.p2p.persistent_peers.is_empty() {
        (PeerMap::new(), vec![])
    } else {
        config::parse_persistent_peers(&node_config.p2p.persistent_peers, &genesis)?
    };

    let listen_addr: litep2p::types::multiaddr::Multiaddr = node_config
        .p2p
        .laddr
        .parse()
        .c(d!("invalid p2p listen address"))?;

    let hotmint::network::service::NetworkServiceHandles {
        service: network_service,
        sink: network_sink,
        msg_rx,
        sync_req_rx,
        sync_resp_rx: _,
        peer_info_rx: _,
        connected_count_rx: _,
        notif_connected_count_rx: mut notif_count_rx,
        mempool_tx_rx: _,
    } = {
        let peer_book_path = home.join("data").join("peer_book.json");
        let peer_book = hotmint::network::peer::PeerBook::load(&peer_book_path)
            .unwrap_or_else(|_| hotmint::network::peer::PeerBook::new(&peer_book_path));
        let peer_book = Arc::new(RwLock::new(peer_book));
        NetworkService::create(hotmint::network::service::NetworkConfig {
            listen_addr,
            peer_map,
            known_addresses,
            keypair: Some(litep2p_keypair),
            peer_book,
            pex_config: {
                let mut pex = node_config.pex.clone();
                pex.private_peer_ids = node_config.p2p.private_peer_ids.clone();
                pex
            },
            relay_consensus: node_config.node.relay_consensus,
            initial_validators: validator_set
                .validators()
                .iter()
                .map(|v| (v.id, v.public_key.clone()))
                .collect(),
            chain_id_hash: state.chain_id_hash,
        })?
    };

    // Application — EVM executor with shared reference for RPC.
    let shared_executor = Arc::new(EvmExecutor::from_genesis(&evm_genesis));
    let app: Box<dyn hotmint::consensus::application::Application> =
        Box::new(SharedExecutor(Arc::clone(&shared_executor)));

    // Ethereum JSON-RPC server.
    let rpc_addr: std::net::SocketAddr = eth_rpc_addr.parse().c(d!("invalid RPC address"))?;
    let rpc_state = Arc::new(EvmRpcState {
        executor: Arc::clone(&shared_executor),
        chain_id: evm_genesis.chain_id,
    });
    tokio::spawn(start_rpc_server(rpc_addr, rpc_state));
    info!(rpc = %eth_rpc_addr, "Ethereum JSON-RPC server listening");

    let sync_sink = network_sink.clone();

    tokio::spawn(async move { network_service.run().await });

    // Sync responder.
    {
        let store = store.clone();
        let sync_sink = sync_sink.clone();
        tokio::spawn(async move {
            use hotmint_types::sync::{SyncRequest, SyncResponse};
            let mut sync_req_rx = sync_req_rx;
            while let Some(req) = sync_req_rx.recv().await {
                let resp = match req.request {
                    SyncRequest::GetStatus => SyncResponse::Status {
                        last_committed_height: Height(0),
                        current_view: ViewNumber(0),
                        epoch: EpochNumber(0),
                    },
                    SyncRequest::GetBlocks {
                        from_height,
                        to_height,
                    } => {
                        let clamped =
                            Height(to_height.as_u64().min(
                                from_height.as_u64() + hotmint_types::sync::MAX_SYNC_BATCH - 1,
                            ));
                        let s = store.read();
                        let blocks = s.get_blocks_in_range(from_height, clamped);
                        let blocks_with_qcs: Vec<_> = blocks
                            .into_iter()
                            .map(|b| {
                                let qc = s.get_commit_qc(b.height);
                                (b, qc)
                            })
                            .collect();
                        drop(s);
                        SyncResponse::Blocks(blocks_with_qcs)
                    }
                    SyncRequest::GetSnapshots => SyncResponse::Snapshots(vec![]),
                    SyncRequest::GetSnapshotChunk {
                        height,
                        chunk_index,
                        ..
                    } => SyncResponse::SnapshotChunk {
                        height,
                        chunk_index,
                        data: vec![],
                    },
                };
                sync_sink.send_sync_response(req.request_id, &resp);
            }
        });
    }

    // Wait for peer connections before starting consensus.
    if !node_config.p2p.persistent_peers.is_empty() {
        info!("waiting for peer connection...");
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);
        loop {
            if *notif_count_rx.borrow() > 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                info!("no peers connected within timeout, starting consensus anyway");
                break;
            }
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_millis(500),
                notif_count_rx.changed(),
            )
            .await;
        }
    }

    // Start consensus engine.
    let signer = Ed25519Signer::new(signing_key, our_vid);
    let engine = ConsensusEngine::new(
        state,
        store,
        Box::new(network_sink),
        app,
        Box::new(signer),
        msg_rx,
        EngineConfig {
            verifier: Box::new(Ed25519Verifier),
            pacemaker: Some(PacemakerConfig {
                base_timeout_ms: node_config.consensus.base_timeout_ms,
                max_timeout_ms: node_config.consensus.max_timeout_ms,
                backoff_multiplier: node_config.consensus.backoff_multiplier,
            }),
            persistence: Some(Box::new(pcs)),
            evidence_store: None,
            wal: None,
            pending_epoch: None,
        },
    );

    info!("consensus engine starting");
    engine.run().await;
    Ok(())
}

/// Default dev genesis with funded test accounts.
fn default_dev_genesis() -> EvmGenesis {
    use hotmint_evm_types::{Address, U256};
    use std::collections::BTreeMap;

    let mut alloc = BTreeMap::new();
    alloc.insert(
        Address::repeat_byte(0xAA),
        hotmint_evm_types::genesis::GenesisAlloc {
            balance: U256::from(10_000u64) * U256::from(1_000_000_000_000_000_000u128),
            nonce: 0,
            code: vec![],
            storage: BTreeMap::new(),
        },
    );
    alloc.insert(
        Address::repeat_byte(0xBB),
        hotmint_evm_types::genesis::GenesisAlloc {
            balance: U256::from(10_000u64) * U256::from(1_000_000_000_000_000_000u128),
            nonce: 0,
            code: vec![],
            storage: BTreeMap::new(),
        },
    );
    EvmGenesis {
        chain_id: 1337,
        alloc,
        gas_limit: 30_000_000,
        base_fee_per_gas: 1_000_000_000,
        coinbase: Address::default(),
        timestamp: 0,
    }
}
