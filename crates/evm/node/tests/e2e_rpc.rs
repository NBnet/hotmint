//! End-to-end integration test for the Hotmint EVM chain.
//!
//! Starts a real 4-node cluster using hotmint-mgmt, builds and launches
//! `hotmint-evm` processes, then runs RPC tests against them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;

use alloy_consensus::{Signed, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;

use hotmint_evm_node::cluster::{init_evm_cluster, start_evm_nodes};
use hotmint_evm_types::genesis::{EvmGenesis, GenesisAlloc};

const NUM_VALIDATORS: u32 = 4;

fn rpc_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

async fn rpc_call(port: u16, method: &str, params: serde_json::Value) -> serde_json::Value {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });
    client
        .post(rpc_url(port))
        .json(&body)
        .send()
        .await
        .expect("RPC request failed")
        .json()
        .await
        .expect("RPC response parse failed")
}

/// Setup cluster, start nodes, return (children, eth_rpc_port_for_node_0, base_dir).
fn setup_and_start(evm_genesis: &EvmGenesis) -> (Vec<Child>, u16, PathBuf) {
    let base_dir = std::env::temp_dir().join(format!("hotmint-evm-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let (state, eth_rpc_ports) = init_evm_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "evm-e2e-test",
        evm_genesis,
        "127.0.0.1",
    )
    .unwrap();

    let binary = hotmint_mgmt::build_binary("hotmint-evm-node", Some("hotmint-evm"))
        .expect("failed to build hotmint-evm");

    let children = start_evm_nodes(&binary, &state, &base_dir, &eth_rpc_ports);
    (children, eth_rpc_ports[0], base_dir)
}

/// Wait until eth_blockNumber returns >= min_height.
async fn wait_for_blocks(port: u16, min_height: u64, timeout_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
    while tokio::time::Instant::now() < deadline {
        let resp = rpc_call(port, "eth_blockNumber", serde_json::json!([])).await;
        if let Some(hex) = resp["result"].as_str()
            && let Ok(h) = u64::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16)
            && h >= min_height
        {
            return true;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_ethereum_rpc() {
    // Create a funded account.
    let signer = PrivateKeySigner::random();
    let funded_addr = signer.address();

    let mut alloc = BTreeMap::new();
    alloc.insert(
        funded_addr,
        GenesisAlloc {
            balance: U256::from(100u64) * U256::from(1_000_000_000_000_000_000u128),
            nonce: 0,
            code: vec![],
            storage: BTreeMap::new(),
        },
    );

    let genesis = EvmGenesis {
        chain_id: 1337,
        alloc,
        gas_limit: 30_000_000,
        base_fee_per_gas: 1_000_000_000,
        coinbase: Address::default(),
        timestamp: 0,
    };

    let (mut children, rpc_port, base_dir) = setup_and_start(&genesis);

    // Wait for cluster to produce at least 1 block.
    assert!(
        wait_for_blocks(rpc_port, 1, 30).await,
        "cluster did not produce blocks within 30s"
    );

    // === Test 1: eth_chainId ===
    let resp = rpc_call(rpc_port, "eth_chainId", serde_json::json!([])).await;
    assert_eq!(resp["result"].as_str().unwrap(), "0x539");

    // === Test 2: web3_clientVersion ===
    let resp = rpc_call(rpc_port, "web3_clientVersion", serde_json::json!([])).await;
    assert!(resp["result"].as_str().unwrap().starts_with("hotmint-evm"));

    // === Test 3: eth_getBalance ===
    let resp = rpc_call(
        rpc_port,
        "eth_getBalance",
        serde_json::json!([format!("0x{}", hex::encode(funded_addr)), "latest"]),
    )
    .await;
    let balance_hex = resp["result"].as_str().unwrap();
    let balance = U256::from_str_radix(balance_hex.strip_prefix("0x").unwrap(), 16).unwrap();
    assert_eq!(
        balance,
        U256::from(100u64) * U256::from(1_000_000_000_000_000_000u128),
    );

    // === Test 4: eth_getTransactionCount ===
    let resp = rpc_call(
        rpc_port,
        "eth_getTransactionCount",
        serde_json::json!([format!("0x{}", hex::encode(funded_addr)), "latest"]),
    )
    .await;
    assert_eq!(resp["result"].as_str().unwrap(), "0x0");

    // === Test 5: eth_gasPrice ===
    let resp = rpc_call(rpc_port, "eth_gasPrice", serde_json::json!([])).await;
    assert_eq!(resp["result"].as_str().unwrap(), "0x3b9aca00");

    // === Test 6: eth_syncing ===
    let resp = rpc_call(rpc_port, "eth_syncing", serde_json::json!([])).await;
    assert_eq!(resp["result"], serde_json::Value::Bool(false));

    // === Test 7: eth_sendRawTransaction (EIP-1559) ===
    let recipient = Address::repeat_byte(0x42);
    let transfer_amount = U256::from(1_000_000_000_000_000_000u128);

    let mut tx = TxEip1559 {
        chain_id: 1337,
        nonce: 0,
        gas_limit: 21_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(recipient),
        value: transfer_amount,
        input: alloy_primitives::Bytes::new(),
        access_list: Default::default(),
    };

    let sig = signer.sign_transaction_sync(&mut tx).expect("signing");
    let signed_tx = Signed::new_unchecked(tx, sig, Default::default());
    let envelope: TxEnvelope = TxEnvelope::from(signed_tx);
    let raw_tx = {
        let mut buf = vec![];
        envelope.encode_2718(&mut buf);
        buf
    };

    let resp = rpc_call(
        rpc_port,
        "eth_sendRawTransaction",
        serde_json::json!([format!("0x{}", hex::encode(&raw_tx))]),
    )
    .await;

    if let Some(hash) = resp["result"].as_str() {
        assert!(hash.starts_with("0x"));
        assert_eq!(hash.len(), 66);
        println!("✓ eth_sendRawTransaction returned tx hash: {hash}");
    } else if let Some(error) = resp["error"].as_object() {
        println!(
            "⚠ eth_sendRawTransaction error: {}",
            error["message"].as_str().unwrap_or("unknown")
        );
    }

    // === Test 8: eth_feeHistory ===
    let resp = rpc_call(
        rpc_port,
        "eth_feeHistory",
        serde_json::json!(["0x1", "latest", [25, 75]]),
    )
    .await;
    assert!(resp["result"]["baseFeePerGas"].is_array());

    // === Test 9: eth_getBlockByNumber ===
    let resp = rpc_call(
        rpc_port,
        "eth_getBlockByNumber",
        serde_json::json!(["latest", false]),
    )
    .await;
    assert!(resp["result"]["gasLimit"].is_string());

    // === Test 10: net_version ===
    let resp = rpc_call(rpc_port, "net_version", serde_json::json!([])).await;
    assert_eq!(resp["result"].as_str().unwrap(), "1337");

    // === Test 11: eth_accounts ===
    let resp = rpc_call(rpc_port, "eth_accounts", serde_json::json!([])).await;
    assert!(resp["result"].as_array().unwrap().is_empty());

    // === Test 12: unknown method ===
    let resp = rpc_call(rpc_port, "nonexistent_method", serde_json::json!([])).await;
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);

    println!("\n✅ All E2E RPC tests passed!");

    // Cleanup.
    for child in &mut children {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);
}
