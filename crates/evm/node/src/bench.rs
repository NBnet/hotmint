//! EVM throughput benchmark using a real multi-process cluster.
//!
//! 1. Initializes a 4-node cluster via hotmint-mgmt
//! 2. Builds and starts `hotmint-evm` node processes
//! 3. Submits pre-signed EIP-1559 transactions via JSON-RPC
//! 4. Measures observed block production and transaction throughput
//! 5. Cleans up

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use alloy_consensus::{Signed, TxEip1559};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_signer_local::PrivateKeySigner;
use serde_json::json;

use hotmint_evm_types::genesis::{EvmGenesis, GenesisAlloc};
use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;
const DURATION_SECS: u64 = 10;
const ETH: u128 = 1_000_000_000_000_000_000;

fn bench_evm_genesis(sender: Address, recipient: Address) -> EvmGenesis {
    let mut alloc = BTreeMap::new();
    alloc.insert(
        sender,
        GenesisAlloc {
            balance: U256::from(1_000_000u64) * U256::from(ETH),
            nonce: 0,
            code: vec![],
            storage: BTreeMap::new(),
        },
    );
    alloc.insert(
        recipient,
        GenesisAlloc {
            balance: U256::ZERO,
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

/// Pre-sign `count` EIP-1559 transfer transactions.
fn presign_txs(
    signer: &PrivateKeySigner,
    recipient: Address,
    chain_id: u64,
    count: usize,
) -> Vec<Vec<u8>> {
    let mut raw_txs = Vec::with_capacity(count);
    for nonce in 0..count as u64 {
        let mut tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit: 21_000,
            max_fee_per_gas: 2_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(recipient),
            value: U256::from(ETH / 1000),
            input: Bytes::new(),
            access_list: Default::default(),
        };
        let sig = signer.sign_transaction_sync(&mut tx).unwrap();
        let signed = Signed::new_unchecked(tx, sig, Default::default());
        let envelope = alloy_consensus::TxEnvelope::from(signed);
        let mut buf = Vec::new();
        envelope.encode_2718(&mut buf);
        raw_txs.push(buf);
    }
    raw_txs
}

/// Query eth_blockNumber via HTTP JSON-RPC.
fn rpc_block_number(rpc_url: &str) -> Option<u64> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let body = json!({
        "jsonrpc": "2.0",
        "method": "eth_blockNumber",
        "params": [],
        "id": 1
    });
    let resp: serde_json::Value = client.post(rpc_url).json(&body).send().ok()?.json().ok()?;
    let hex = resp["result"].as_str()?;
    u64::from_str_radix(hex.strip_prefix("0x")?, 16).ok()
}

/// Submit a raw transaction via eth_sendRawTransaction.
fn rpc_send_raw_tx(rpc_url: &str, raw_tx: &[u8]) -> bool {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "eth_sendRawTransaction",
        "params": [format!("0x{}", hex::encode(raw_tx))],
        "id": 1
    });
    client.post(rpc_url).json(&body).send().is_ok()
}

fn setup_cluster(base_dir: &Path, evm_genesis: &EvmGenesis) -> ruc::Result<()> {
    // Find free ports for p2p and rpc.
    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        base_dir,
        NUM_VALIDATORS,
        "evm-bench",
        p2p_base,
        rpc_base,
        "127.0.0.1",
    )?;

    // Write EVM genesis to each validator's config dir.
    let evm_genesis_json = serde_json::to_string_pretty(evm_genesis).map_err(|e| ruc::eg!(e))?;
    for i in 0..NUM_VALIDATORS {
        let config_dir = base_dir.join(format!("v{i}")).join("config");
        std::fs::write(config_dir.join("evm-genesis.json"), &evm_genesis_json)
            .map_err(|e| ruc::eg!(e))?;
    }

    Ok(())
}

fn start_evm_nodes(base_dir: &Path, binary: &Path, eth_rpc_ports: &[u16]) -> Vec<Child> {
    let state = cluster::ClusterState::load(base_dir).unwrap();
    let mut children = Vec::new();

    for (i, v) in state.validators.iter().enumerate() {
        let log_file = base_dir.join(format!("v{}.log", v.id));
        let log = std::fs::File::create(&log_file).unwrap();
        let log_err = log.try_clone().unwrap();

        let child = Command::new(binary)
            .arg("--home")
            .arg(&v.home_dir)
            .arg("--rpc-addr")
            .arg(format!("127.0.0.1:{}", eth_rpc_ports[i]))
            .stdout(log)
            .stderr(log_err)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn V{}: {e}", v.id));

        children.push(child);
        // Stagger startup to avoid Noise handshake collisions.
        if i < state.validators.len() - 1 {
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    children
}

fn wait_for_blocks(rpc_url: &str, min_blocks: u64, timeout_secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        if let Some(height) = rpc_block_number(rpc_url)
            && height >= min_blocks
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn run_bench(label: &str, txs_to_submit: usize) {
    let eth_signer = PrivateKeySigner::random();
    let sender = eth_signer.address();
    let recipient = Address::repeat_byte(0x42);
    let evm_genesis = bench_evm_genesis(sender, recipient);

    let base_dir = std::env::temp_dir().join(format!(
        "hotmint-evm-bench-{}-{}",
        std::process::id(),
        label.replace(' ', "-")
    ));
    let _ = std::fs::remove_dir_all(&base_dir);

    // Setup cluster.
    setup_cluster(&base_dir, &evm_genesis).unwrap();
    let _state = cluster::ClusterState::load(&base_dir).unwrap();

    // Find or build the hotmint-evm binary.
    let binary = hotmint_mgmt::build_binary("hotmint-evm-node", Some("hotmint-evm"))
        .expect("failed to build hotmint-evm binary");

    // Assign eth_* RPC ports (distinct from the hotmint RPC ports).
    let eth_rpc_ports = hotmint_mgmt::find_free_ports(NUM_VALIDATORS as usize);

    // Start nodes.
    let mut children = start_evm_nodes(&base_dir, &binary, &eth_rpc_ports);
    let rpc_url = format!("http://127.0.0.1:{}", eth_rpc_ports[0]);

    // Wait for cluster to start producing blocks.
    println!("  Waiting for cluster to start...");
    if !wait_for_blocks(&rpc_url, 1, 30) {
        eprintln!("  ERROR: cluster did not produce blocks within 30s");
        for child in &mut children {
            let _ = child.kill();
        }
        let _ = std::fs::remove_dir_all(&base_dir);
        return;
    }

    // Pre-sign transactions.
    let raw_txs = presign_txs(&eth_signer, recipient, 1337, txs_to_submit);

    // Get starting block number.
    let start_block = rpc_block_number(&rpc_url).unwrap_or(0);

    // Submit transactions as fast as possible.
    let submit_start = Instant::now();
    let mut submitted = 0u64;
    for raw_tx in &raw_txs {
        if rpc_send_raw_tx(&rpc_url, raw_tx) {
            submitted += 1;
        }
    }
    let submit_elapsed = submit_start.elapsed();

    // Wait for transactions to be included in blocks.
    std::thread::sleep(Duration::from_secs(DURATION_SECS));
    let end_block = rpc_block_number(&rpc_url).unwrap_or(start_block);

    let blocks_produced = end_block.saturating_sub(start_block);
    let elapsed_total = submit_elapsed + Duration::from_secs(DURATION_SECS);
    let blocks_per_sec = blocks_produced as f64 / elapsed_total.as_secs_f64();
    let tx_per_sec = submitted as f64 / elapsed_total.as_secs_f64();
    let ms_per_block = if blocks_produced > 0 {
        elapsed_total.as_millis() as f64 / blocks_produced as f64
    } else {
        f64::INFINITY
    };

    println!("  Config: {label}");
    println!("    {NUM_VALIDATORS} validators (separate processes), real litep2p networking");
    println!(
        "    Submitted: {submitted} EIP-1559 transfers in {:.1}s",
        submit_elapsed.as_secs_f64()
    );
    println!(
        "    Result: {blocks_per_sec:.1} blocks/sec, {tx_per_sec:.0} tx/sec, {ms_per_block:.1} ms/block"
    );
    println!("    Total: {blocks_produced} blocks (height {start_block}→{end_block})");
    println!();

    // Cleanup.
    for child in &mut children {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);
}

fn main() {
    println!("=== Hotmint EVM Throughput Benchmark (multi-process, real P2P) ===\n");

    run_bench("100 txs", 100);
    run_bench("500 txs", 500);

    println!("Done.");
}
