//! Hotmint EVM Node — production-grade EVM-compatible chain.
//!
//! Runs a validator or fullnode with real P2P networking (litep2p).
//! Uses the same `--home` config layout as `hotmint-node` plus an
//! additional `config/evm-genesis.json` for EVM-specific settings.
//!
//! Usage:
//!   hotmint-evm init --home /path/to/home
//!   hotmint-evm node --home /path/to/home [--rpc-addr 127.0.0.1:8545]

use ruc::*;

use std::future;
use std::process;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::sync::{RwLock, watch};
use tracing::{Level, error, info};

use hotmint::config::{self, GenesisDoc, NodeConfig, NodeKey, NodeMode, PrivValidatorKey};
use hotmint::consensus::engine::{ConsensusEngine, EngineConfig};
use hotmint::consensus::pacemaker::PacemakerConfig;
use hotmint::consensus::state::ConsensusState;
use hotmint::consensus::store::{BlockStore, SharedStoreAdapter};
use hotmint::consensus::sync::sync_to_tip;
use hotmint::crypto::{Ed25519Signer, Ed25519Verifier};
use hotmint::network::service::{NetworkService, PeerMap};
use hotmint::prelude::*;
use hotmint::storage::block_store::VsdbBlockStore;
use hotmint::storage::consensus_state::PersistentConsensusState;

use hotmint_evm_execution::{EvmExecutor, SharedExecutor};
use hotmint_evm_rpc::{EvmRpcState, start_rpc_server};
use hotmint_evm_types::genesis::EvmGenesis;

/// Hotmint EVM Node — production-grade EVM-compatible chain.
#[derive(Parser)]
#[command(name = "hotmint-evm", about = "Hotmint EVM-compatible chain node")]
struct Cli {
    /// Path to hotmint home directory (contains config/, data/).
    #[arg(long, global = true)]
    home: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new node directory with keys and default config.
    Init,
    /// Run the EVM node.
    Node {
        /// Override Ethereum JSON-RPC listen address (host:port).
        #[arg(long)]
        rpc_addr: Option<String>,
        /// Override P2P listen address (multiaddr).
        #[arg(long)]
        p2p_laddr: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let home = cli
        .home
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".hotmint-evm")
        });

    match cli.command {
        Command::Init => {
            if let Err(e) = config::init_node_dir(&home) {
                eprintln!("Error: {e}");
                process::exit(1);
            }
            // Also write a default evm-genesis.json if it doesn't exist.
            let evm_genesis_path = home.join("config").join("evm-genesis.json");
            if !evm_genesis_path.exists() {
                let genesis = default_dev_genesis();
                let json = serde_json::to_string_pretty(&genesis).unwrap();
                std::fs::write(&evm_genesis_path, json).unwrap();
                println!("Wrote default evm-genesis.json");
            }
            println!("Initialized EVM node directory at {}", home.display());
        }
        Command::Node {
            rpc_addr,
            p2p_laddr,
        } => {
            tracing_subscriber::fmt()
                .with_max_level(Level::INFO)
                .with_target(false)
                .init();

            if let Err(e) = run_node(&home, rpc_addr, p2p_laddr).await {
                eprintln!("Fatal: {e}");
                process::exit(1);
            }
        }
    }
}

async fn run_node(
    home: &std::path::Path,
    rpc_addr_override: Option<String>,
    p2p_laddr_override: Option<String>,
) -> Result<()> {
    let config_dir = home.join("config");
    let data_dir = home.join("data");

    // Load standard hotmint config files.
    let mut node_config =
        NodeConfig::load(&config_dir.join("config.toml")).c(d!("failed to load config.toml"))?;
    let priv_key = PrivValidatorKey::load(&config_dir.join("priv_validator_key.json"))
        .c(d!("failed to load priv_validator_key.json"))?;
    let signing_key = priv_key.to_signing_key()?;
    let node_key =
        NodeKey::load(&config_dir.join("node_key.json")).c(d!("failed to load node_key.json"))?;
    let litep2p_keypair = node_key.to_litep2p_keypair()?;

    // Apply CLI overrides.
    if let Some(pl) = p2p_laddr_override {
        node_config.p2p.laddr = pl;
    }
    let eth_rpc_addr = rpc_addr_override.unwrap_or_else(|| "127.0.0.1:8545".to_string());

    let genesis =
        GenesisDoc::load(&config_dir.join("genesis.json")).c(d!("failed to load genesis.json"))?;
    let validator_set = genesis.to_validator_set()?;

    // Load EVM genesis.
    let evm_genesis_path = config_dir.join("evm-genesis.json");
    let evm_genesis = if evm_genesis_path.exists() {
        EvmGenesis::load(&evm_genesis_path).c(d!("failed to load evm-genesis.json"))?
    } else {
        info!("No evm-genesis.json found, using default dev genesis");
        default_dev_genesis()
    };

    // Identify this validator. If not in genesis, run as fullnode.
    let our_pk_hex = &priv_key.public_key;
    let is_fullnode;
    let our_vid = if let Some(gv) = genesis
        .validators
        .iter()
        .find(|v| &v.public_key == our_pk_hex)
    {
        is_fullnode = node_config.node.mode == NodeMode::Fullnode;
        ValidatorId(gv.id)
    } else {
        is_fullnode = true;
        let sentinel = ValidatorId(u64::MAX);
        assert!(
            !validator_set.validators().iter().any(|v| v.id == sentinel),
            "genesis contains a validator with ID u64::MAX which collides with the fullnode sentinel"
        );
        sentinel
    };

    if is_fullnode {
        info!(
            node_id = our_vid.0,
            chain_id = evm_genesis.chain_id,
            validators = validator_set.validator_count(),
            "starting hotmint-evm fullnode (sync-only, no consensus participation)"
        );
    } else {
        info!(
            validator_id = %our_vid,
            chain_id = evm_genesis.chain_id,
            accounts = evm_genesis.alloc.len(),
            gas_limit = evm_genesis.gas_limit,
            validators = validator_set.validator_count(),
            "=== Hotmint EVM Node ==="
        );
    }

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
    let mut engine_state_epoch = state.current_epoch.clone();
    let mut engine_state_height = state.last_committed_height;
    let mut engine_state_app_hash = state.last_app_hash;
    let mut engine_state_pending_epoch = None;
    if let Some(epoch) = pcs.load_current_epoch() {
        state.validator_set = epoch.validator_set.clone();
        state.current_epoch = epoch.clone();
        engine_state_epoch = epoch;
    }

    // Sync status watch channel.
    let (sync_status_tx, _sync_status_rx) =
        watch::channel(hotmint::api::rpc::ConsensusStatus::new(
            state.current_view.as_u64(),
            state.last_committed_height.as_u64(),
            state.current_epoch.number.as_u64(),
            state.validator_set.validator_count(),
            state.current_epoch.start_view.as_u64(),
        ));

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
        mut sync_resp_rx,
        peer_info_rx: _,
        connected_count_rx: _,
        notif_connected_count_rx: mut notif_count_rx,
        mut mempool_tx_rx,
    } = {
        let peer_book_path = home.join("data").join("peer_book.json");
        let peer_book = hotmint::network::peer::PeerBook::load(&peer_book_path)
            .unwrap_or_else(|_| hotmint::network::peer::PeerBook::new(&peer_book_path));
        let peer_book = Arc::new(RwLock::new(peer_book));
        NetworkService::create(hotmint::network::service::NetworkConfig {
            listen_addr,
            peer_map: peer_map.clone(),
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

    // Application — EVM executor.
    let shared_executor = Arc::new(EvmExecutor::from_genesis(&evm_genesis));
    shared_executor.setup_nonce_fn();
    let app: Arc<dyn hotmint::consensus::application::Application> =
        Arc::new(SharedExecutor(Arc::clone(&shared_executor)));

    // Ethereum JSON-RPC server (conditional on serve_rpc).
    let rpc_handle: tokio::task::JoinHandle<()> = if node_config.node.serve_rpc {
        let rpc_addr: std::net::SocketAddr = eth_rpc_addr.parse().c(d!("invalid RPC address"))?;
        let rpc_state = Arc::new(EvmRpcState {
            executor: Arc::clone(&shared_executor),
            chain_id: evm_genesis.chain_id,
            network_sink: Some(Arc::new(network_sink.clone())),
        });
        info!(rpc = %eth_rpc_addr, "Ethereum JSON-RPC server listening");
        tokio::spawn(start_rpc_server(rpc_addr, rpc_state))
    } else {
        info!("Ethereum JSON-RPC server disabled by config (serve_rpc = false)");
        tokio::spawn(future::pending())
    };

    // Mempool gossip: receive txs from peers.
    {
        use hotmint_evm_execution::EvmMempoolAdapter;
        use hotmint_mempool::MempoolAdapter;

        let gossip_mempool = Arc::new(EvmMempoolAdapter {
            txpool: Arc::clone(&shared_executor.txpool),
        });
        let gossip_app = app.clone();
        tokio::spawn(async move {
            while let Some(tx_bytes) = mempool_tx_rx.recv().await {
                let result = gossip_app.validate_tx(&tx_bytes, None);
                if result.valid {
                    let _ = gossip_mempool
                        .add_tx(tx_bytes, result.priority, result.gas_wanted)
                        .await;
                }
            }
        });
    }

    let sync_sink = network_sink.clone();

    let network_handle = tokio::spawn(async move { network_service.run().await });

    // Sync responder (conditional on serve_sync).
    let sync_responder_handle: tokio::task::JoinHandle<()> = if node_config.node.serve_sync {
        let store = store.clone();
        let sync_status_rx = sync_status_tx.subscribe();
        let sync_sink = sync_sink.clone();
        let responder_app = app.clone();
        tokio::spawn(async move {
            let mut sync_req_rx = sync_req_rx;
            use hotmint_types::sync::{SyncRequest, SyncResponse};
            while let Some(req) = sync_req_rx.recv().await {
                let resp = match req.request {
                    SyncRequest::GetStatus => {
                        let s = *sync_status_rx.borrow();
                        SyncResponse::Status {
                            last_committed_height: Height(s.last_committed_height),
                            current_view: ViewNumber(s.current_view),
                            epoch: EpochNumber(s.epoch_number),
                        }
                    }
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
                    SyncRequest::GetSnapshots => {
                        SyncResponse::Snapshots(responder_app.list_snapshots())
                    }
                    SyncRequest::GetSnapshotChunk {
                        height,
                        chunk_index,
                    } => {
                        let data = responder_app.load_snapshot_chunk(height, chunk_index);
                        SyncResponse::SnapshotChunk {
                            height,
                            chunk_index,
                            data,
                        }
                    }
                };
                sync_sink.send_sync_response(req.request_id, &resp);
            }
        })
    } else {
        info!("sync responder disabled by config (serve_sync = false)");
        tokio::spawn(future::pending())
    };

    // Block sync: catch up from peers before starting consensus.
    let sync_peers: Vec<_> = peer_map
        .validator_to_peer
        .iter()
        .map(|(&vid, &pid)| (vid, pid))
        .collect();
    if !sync_peers.is_empty() {
        info!("waiting for peer connection before sync...");
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);
        loop {
            if *notif_count_rx.borrow() > 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                info!("no peers connected within timeout, skipping sync");
                break;
            }
            let _ = tokio::time::timeout(
                tokio::time::Duration::from_millis(500),
                notif_count_rx.changed(),
            )
            .await;
        }

        if *notif_count_rx.borrow() > 0 {
            use hotmint_types::sync::SyncRequest;

            let sync_app: Box<dyn hotmint::consensus::application::Application> =
                Box::new(SharedExecutor(Arc::clone(&shared_executor)));

            let mut synced = false;
            for (vid, peer_id) in &sync_peers {
                let bridge_sink = sync_sink.clone();
                let pid = *peer_id;
                let (sync_tx, mut sync_bridge_rx) = tokio::sync::mpsc::channel::<SyncRequest>(16);

                let bridge = tokio::spawn(async move {
                    while let Some(req) = sync_bridge_rx.recv().await {
                        bridge_sink.send_sync_request(pid, &req);
                    }
                });

                while sync_resp_rx.try_recv().is_ok() {}

                info!("starting block sync with V{}", vid.0);
                let mut sync_store = SharedStoreAdapter(store.clone());
                let mut sync_state = hotmint::consensus::sync::SyncState {
                    store: &mut sync_store,
                    app: sync_app.as_ref(),
                    current_epoch: &mut engine_state_epoch,
                    last_committed_height: &mut engine_state_height,
                    last_app_hash: &mut engine_state_app_hash,
                    chain_id_hash: &state.chain_id_hash,
                    pending_epoch: &mut engine_state_pending_epoch,
                };
                match sync_to_tip(&mut sync_state, &sync_tx, &mut sync_resp_rx).await {
                    Ok(()) => {
                        bridge.abort();
                        synced = true;
                        break;
                    }
                    Err(e) => {
                        info!(%e, peer = vid.0, "sync from peer failed, trying next");
                        bridge.abort();
                    }
                }
            }
            if !synced && !sync_peers.is_empty() {
                info!("all sync peers failed, continuing from current state");
            }

            // Apply synced state.
            state.current_epoch = engine_state_epoch;
            state.validator_set = state.current_epoch.validator_set.clone();
            state.last_committed_height = engine_state_height;
            state.last_app_hash = engine_state_app_hash;
            if state.last_committed_height.as_u64() > 0 {
                let tip_view = {
                    let s = store.read();
                    s.get_block_by_height(state.last_committed_height)
                        .map(|b| b.view)
                        .unwrap_or(state.current_view)
                };
                if tip_view > state.current_view {
                    state.current_view = ViewNumber(tip_view.as_u64() + 1);
                }
            }
        }
    } else {
        // No persistent peers — wait briefly for connections.
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
    }

    // Start consensus engine.
    let signer = Ed25519Signer::new(signing_key, our_vid);
    let engine = ConsensusEngine::new(
        state,
        store,
        Box::new(network_sink),
        Box::new(SharedExecutor(Arc::clone(&shared_executor))),
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
            evidence_store: Some(Box::new(
                hotmint::storage::evidence_store::PersistentEvidenceStore::open(&data_dir)
                    .unwrap_or_else(|e| panic!("failed to open evidence store: {e}")),
            )),
            wal: Some(Box::new(
                hotmint::storage::wal::ConsensusWal::open(&data_dir)
                    .expect("failed to open consensus WAL"),
            )),
            pending_epoch: engine_state_pending_epoch,
        },
    );

    info!("consensus engine starting");

    // Supervisor: run engine + handle shutdown.
    tokio::select! {
        () = engine.run() => {},
        res = network_handle => {
            match res {
                Ok(()) => error!("network service exited unexpectedly"),
                Err(e) => error!("network service panicked: {e}"),
            }
            process::exit(1);
        }
        res = rpc_handle => {
            match res {
                Ok(()) => error!("RPC server exited unexpectedly"),
                Err(e) => error!("RPC server panicked: {e}"),
            }
            process::exit(1);
        }
        res = sync_responder_handle => {
            match res {
                Ok(()) => error!("sync responder exited unexpectedly"),
                Err(e) => error!("sync responder panicked: {e}"),
            }
            process::exit(1);
        }
        _ = tokio::signal::ctrl_c() => {
            info!("received shutdown signal, exiting...");
        }
        _ = async {
            #[cfg(unix)]
            {
                let mut sigterm = tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate()
                ).expect("failed to register SIGTERM handler");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        } => {
            info!("received SIGTERM, shutting down...");
        }
    }

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
