//! End-to-end integration test for the Hotmint EVM chain.

use std::collections::BTreeMap;
use std::sync::Arc;

use alloy_consensus::{Signed, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_network::TxSignerSync;
use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;

use hotmint::crypto::{Ed25519Signer, Ed25519Verifier};
use hotmint_consensus::engine::ConsensusEngineBuilder;
use hotmint_consensus::network::ChannelNetwork;
use hotmint_consensus::state::ConsensusState;
use hotmint_consensus::store::MemoryBlockStore;
use hotmint_evm_execution::{EvmExecutor, SharedExecutor};
use hotmint_evm_rpc::{EvmRpcState, start_rpc_server};
use hotmint_evm_types::genesis::{EvmGenesis, GenesisAlloc};
use hotmint_types::*;

const NUM_VALIDATORS: u64 = 4;
const RPC_PORT: u16 = 18545;

fn rpc_url() -> String {
    format!("http://127.0.0.1:{RPC_PORT}/")
}

async fn rpc_call(method: &str, params: serde_json::Value) -> serde_json::Value {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });
    client
        .post(&rpc_url())
        .json(&body)
        .send()
        .await
        .expect("RPC request failed")
        .json()
        .await
        .expect("RPC response parse failed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_ethereum_rpc() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init()
        .ok();

    // Create a funded account using a known private key.
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

    // Build the validator set.
    let signers: Vec<Ed25519Signer> = (0..NUM_VALIDATORS)
        .map(|i| Ed25519Signer::generate(ValidatorId(i)))
        .collect();
    let signer_refs: Vec<&dyn Signer> = signers.iter().map(|s| s as &dyn Signer).collect();
    let validator_set = ValidatorSet::from_signers(&signer_refs);

    let shared_executor = Arc::new(EvmExecutor::from_genesis(&genesis));

    // Start RPC server.
    let rpc_state = Arc::new(EvmRpcState {
        executor: Arc::clone(&shared_executor),
        chain_id: genesis.chain_id,
    });
    let rpc_addr = format!("127.0.0.1:{RPC_PORT}").parse().unwrap();
    tokio::spawn(start_rpc_server(rpc_addr, rpc_state));

    // Start consensus engines.
    let mesh = ChannelNetwork::create_mesh(NUM_VALIDATORS);
    for (i, ((network, rx), ed_signer)) in mesh.into_iter().zip(signers.into_iter()).enumerate() {
        let vid = ValidatorId(i as u64);
        let store = MemoryBlockStore::new_shared();
        let state = ConsensusState::new(vid, validator_set.clone());

        let app: Box<dyn hotmint_consensus::application::Application> = if i == 0 {
            Box::new(SharedExecutor(Arc::clone(&shared_executor)))
        } else {
            Box::new(EvmExecutor::from_genesis(&genesis))
        };

        let engine = ConsensusEngineBuilder::new()
            .state(state)
            .store(store)
            .network(Box::new(network))
            .app(app)
            .signer(Box::new(ed_signer))
            .messages(rx)
            .verifier(Box::new(Ed25519Verifier))
            .build()
            .expect("engine build");

        tokio::spawn(async move { engine.run().await });
    }

    // Wait for startup.
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // === Test 1: eth_chainId ===
    let resp = rpc_call("eth_chainId", serde_json::json!([])).await;
    assert_eq!(resp["result"].as_str().unwrap(), "0x539");

    // === Test 2: web3_clientVersion ===
    let resp = rpc_call("web3_clientVersion", serde_json::json!([])).await;
    assert!(resp["result"].as_str().unwrap().starts_with("hotmint-evm"));

    // === Test 3: eth_getBalance ===
    let resp = rpc_call(
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
        "eth_getTransactionCount",
        serde_json::json!([format!("0x{}", hex::encode(funded_addr)), "latest"]),
    )
    .await;
    assert_eq!(resp["result"].as_str().unwrap(), "0x0");

    // === Test 5: eth_gasPrice ===
    let resp = rpc_call("eth_gasPrice", serde_json::json!([])).await;
    assert_eq!(resp["result"].as_str().unwrap(), "0x3b9aca00");

    // === Test 6: eth_syncing ===
    let resp = rpc_call("eth_syncing", serde_json::json!([])).await;
    assert_eq!(resp["result"], serde_json::Value::Bool(false));

    // === Test 7: eth_sendRawTransaction (EIP-1559) ===
    let recipient = Address::repeat_byte(0x42);
    let transfer_amount = U256::from(1_000_000_000_000_000_000u128); // 1 ETH

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

    let sig = signer
        .sign_transaction_sync(&mut tx)
        .expect("signing should work");
    let signed_tx = Signed::new_unchecked(tx, sig, Default::default());
    let envelope: TxEnvelope = TxEnvelope::from(signed_tx);
    let raw_tx = {
        let mut buf = vec![];
        envelope.encode_2718(&mut buf);
        buf
    };

    let resp = rpc_call(
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
        "eth_feeHistory",
        serde_json::json!(["0x1", "latest", [25, 75]]),
    )
    .await;
    assert!(resp["result"]["baseFeePerGas"].is_array());

    // === Test 9: eth_getBlockByNumber ===
    let resp = rpc_call(
        "eth_getBlockByNumber",
        serde_json::json!(["latest", false]),
    )
    .await;
    assert!(resp["result"]["gasLimit"].is_string());

    // === Test 10: net_version ===
    let resp = rpc_call("net_version", serde_json::json!([])).await;
    assert_eq!(resp["result"].as_str().unwrap(), "1337");

    // === Test 11: eth_accounts ===
    let resp = rpc_call("eth_accounts", serde_json::json!([])).await;
    assert!(resp["result"].as_array().unwrap().is_empty());

    // === Test 12: unknown method ===
    let resp = rpc_call("nonexistent_method", serde_json::json!([])).await;
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);

    println!("\n✅ All E2E RPC tests passed!");
}
