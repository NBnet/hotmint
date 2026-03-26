use clap::Parser;
use tracing::{Level, info};

use hotmint_consensus::engine::ConsensusEngineBuilder;
use hotmint_consensus::network::ChannelNetwork;
use hotmint_consensus::state::ConsensusState;
use hotmint_consensus::store::MemoryBlockStore;
use hotmint::crypto::{Ed25519Signer, Ed25519Verifier};
use hotmint_evm_execution::EvmExecutor;
use hotmint_evm_types::genesis::EvmGenesis;
use hotmint_types::*;

const NUM_VALIDATORS: u64 = 4;

/// Hotmint EVM Node — production-grade EVM-compatible chain.
#[derive(Parser)]
#[command(name = "hotmint-evm", about = "Hotmint EVM-compatible chain node")]
struct Cli {
    /// Path to EVM genesis JSON file.
    #[arg(long, default_value = "evm-genesis.json")]
    genesis: String,

    /// Run duration in seconds (0 = run forever).
    #[arg(long, default_value = "30")]
    duration: u64,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    // Load or use default genesis.
    let genesis = if std::path::Path::new(&cli.genesis).exists() {
        EvmGenesis::load(std::path::Path::new(&cli.genesis)).unwrap_or_else(|e| {
            eprintln!("Failed to load genesis: {e}");
            std::process::exit(1);
        })
    } else {
        info!("No genesis file found, using default with funded test accounts");
        default_dev_genesis()
    };

    info!(
        chain_id = genesis.chain_id,
        accounts = genesis.alloc.len(),
        gas_limit = genesis.gas_limit,
        "=== Hotmint EVM Chain ==="
    );

    let signers: Vec<Ed25519Signer> = (0..NUM_VALIDATORS)
        .map(|i| Ed25519Signer::generate(ValidatorId(i)))
        .collect();

    let signer_refs: Vec<&dyn Signer> = signers.iter().map(|s| s as &dyn Signer).collect();
    let validator_set = ValidatorSet::from_signers(&signer_refs);

    info!(
        validators = validator_set.validator_count(),
        quorum = validator_set.quorum_threshold(),
        "validator set initialized"
    );

    let mesh = ChannelNetwork::create_mesh(NUM_VALIDATORS);
    let mut handles = Vec::new();

    for (i, ((network, rx), signer)) in mesh.into_iter().zip(signers.into_iter()).enumerate() {
        let vid = ValidatorId(i as u64);
        let store = MemoryBlockStore::new_shared();
        let state = ConsensusState::new(vid, validator_set.clone());

        let executor = EvmExecutor::from_genesis(&genesis);

        let engine = ConsensusEngineBuilder::new()
            .state(state)
            .store(store)
            .network(Box::new(network))
            .app(Box::new(executor))
            .signer(Box::new(signer))
            .messages(rx)
            .verifier(Box::new(Ed25519Verifier))
            .build()
            .expect("all required fields set");

        handles.push(tokio::spawn(async move { engine.run().await }));
    }

    info!("All validators spawned, EVM chain running...\n");

    if cli.duration > 0 {
        tokio::time::sleep(tokio::time::Duration::from_secs(cli.duration)).await;
        info!("\n=== Hotmint EVM Chain stopped after {}s ===", cli.duration);
    } else {
        // Run forever.
        std::future::pending::<()>().await;
    }
}

/// Default dev genesis with funded test accounts.
fn default_dev_genesis() -> EvmGenesis {
    use hotmint_evm_types::{Address, U256};
    use std::collections::BTreeMap;

    let mut alloc = BTreeMap::new();
    // Dev account 0: 10000 ETH
    alloc.insert(
        Address::repeat_byte(0xAA),
        hotmint_evm_types::genesis::GenesisAlloc {
            balance: U256::from(10_000u64) * U256::from(1_000_000_000_000_000_000u128),
            nonce: 0,
            code: vec![],
            storage: BTreeMap::new(),
        },
    );
    // Dev account 1: 10000 ETH
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
