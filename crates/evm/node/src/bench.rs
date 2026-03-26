//! EVM throughput benchmark using a real multi-process cluster.
//!
//! 1. Initializes a 4-node cluster via hotmint-mgmt
//! 2. Builds and starts `hotmint-evm` node processes
//! 3. Submits pre-signed EIP-1559 transactions via JSON-RPC
//! 4. Polls until all transactions are confirmed on-chain (nonce-based)
//! 5. Reports true TPS (confirmed tx / wall-clock time)
//! 6. Cleans up

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use alloy_consensus::{Signed, TxEip1559};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_signer_local::PrivateKeySigner;
use serde_json::json;

use hotmint_evm_node::cluster::{init_evm_cluster, start_evm_nodes};
use hotmint_evm_types::genesis::{EvmGenesis, GenesisAlloc};

const NUM_VALIDATORS: u32 = 4;
const ETH: u128 = 1_000_000_000_000_000_000;
/// Maximum time to wait for all txs to be confirmed.
const CONFIRM_TIMEOUT_SECS: u64 = 120;

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

fn rpc_json(rpc_url: &str, method: &str, params: serde_json::Value) -> Option<serde_json::Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let body = json!({ "jsonrpc": "2.0", "method": method, "params": params, "id": 1 });
    let resp: serde_json::Value = client.post(rpc_url).json(&body).send().ok()?.json().ok()?;
    Some(resp)
}

fn rpc_block_number(rpc_url: &str) -> Option<u64> {
    let resp = rpc_json(rpc_url, "eth_blockNumber", json!([]))?;
    let hex = resp["result"].as_str()?;
    u64::from_str_radix(hex.strip_prefix("0x")?, 16).ok()
}

fn rpc_get_nonce(rpc_url: &str, addr: &Address) -> Option<u64> {
    let resp = rpc_json(
        rpc_url,
        "eth_getTransactionCount",
        json!([format!("0x{}", hex::encode(addr)), "latest"]),
    )?;
    let hex = resp["result"].as_str()?;
    u64::from_str_radix(hex.strip_prefix("0x")?, 16).ok()
}

fn rpc_send_raw_tx(rpc_url: &str, raw_tx: &[u8]) -> bool {
    rpc_json(
        rpc_url,
        "eth_sendRawTransaction",
        json!([format!("0x{}", hex::encode(raw_tx))]),
    )
    .is_some()
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

/// Wait until the sender's on-chain nonce reaches `expected_nonce`.
/// Returns actual confirmed nonce and elapsed time since `start`.
fn wait_for_confirmations(
    rpc_url: &str,
    sender: &Address,
    expected_nonce: u64,
    start: Instant,
    timeout_secs: u64,
) -> (u64, Duration) {
    let deadline = start + Duration::from_secs(timeout_secs);
    let mut confirmed = 0u64;
    while Instant::now() < deadline {
        if let Some(nonce) = rpc_get_nonce(rpc_url, sender) {
            confirmed = nonce;
            if nonce >= expected_nonce {
                return (confirmed, start.elapsed());
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    (confirmed, start.elapsed())
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
    let (state, eth_rpc_ports) = init_evm_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "evm-bench",
        &evm_genesis,
        "127.0.0.1",
    )
    .unwrap();

    // Find or build the hotmint-evm binary.
    let binary = hotmint_mgmt::build_binary("hotmint-evm-node", Some("hotmint-evm"))
        .expect("failed to build hotmint-evm binary");

    // Start nodes.
    let mut children = start_evm_nodes(&binary, &state, &base_dir, &eth_rpc_ports);
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

    // Record starting state.
    let start_block = rpc_block_number(&rpc_url).unwrap_or(0);

    // Submit all transactions as fast as possible.
    let bench_start = Instant::now();
    let mut submitted = 0u64;
    for raw_tx in &raw_txs {
        if rpc_send_raw_tx(&rpc_url, raw_tx) {
            submitted += 1;
        }
    }
    let submit_elapsed = bench_start.elapsed();

    // Wait for all txs to be confirmed on-chain (nonce reaches submitted count).
    let (confirmed, total_elapsed) = wait_for_confirmations(
        &rpc_url,
        &sender,
        submitted,
        bench_start,
        CONFIRM_TIMEOUT_SECS,
    );

    let end_block = rpc_block_number(&rpc_url).unwrap_or(start_block);
    let blocks_produced = end_block.saturating_sub(start_block);

    // TPS = confirmed tx / total wall-clock time (submit + confirmation).
    let tps = if total_elapsed.as_secs_f64() > 0.0 {
        confirmed as f64 / total_elapsed.as_secs_f64()
    } else {
        0.0
    };

    println!("  Config: {label}");
    println!("    {NUM_VALIDATORS} validators (separate processes), real litep2p networking");
    println!(
        "    Submitted:  {submitted} txs in {:.2}s ({:.0} submit/s)",
        submit_elapsed.as_secs_f64(),
        submitted as f64 / submit_elapsed.as_secs_f64().max(0.001),
    );
    println!(
        "    Confirmed:  {confirmed}/{submitted} txs in {:.2}s",
        total_elapsed.as_secs_f64(),
    );
    println!("    Throughput: {tps:.1} tx/s (confirmed on-chain)");
    println!("    Blocks:     {blocks_produced} (height {start_block}→{end_block})",);
    if confirmed < submitted {
        println!(
            "    WARNING: {}/{submitted} txs NOT confirmed within timeout",
            submitted - confirmed
        );
    }
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
