//! Ethereum JSON-RPC server for Hotmint EVM chains.
//!
//! Implements eth_*, net_*, web3_* methods compatible with MetaMask,
//! Foundry, and Hardhat. Runs as a standalone axum server on port 8545.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use alloy_primitives::{Address, B256, U256};
use hotmint_consensus::application::Application;
use hotmint_evm_execution::EvmExecutor;

/// Shared state for the EVM RPC server.
pub struct EvmRpcState {
    pub executor: Arc<EvmExecutor>,
    pub chain_id: u64,
}

/// JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: Option<String>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn err(id: serde_json::Value, code: i64, msg: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: msg.to_string(),
            }),
            id,
        }
    }
}

/// Start the EVM JSON-RPC server.
pub async fn start_rpc_server(addr: SocketAddr, state: Arc<EvmRpcState>) {
    let app = Router::new().route("/", post(handle_rpc)).with_state(state);

    info!(%addr, "starting EVM JSON-RPC server");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_rpc(
    State(state): State<Arc<EvmRpcState>>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let response = dispatch(&state, &req);
    Json(response)
}

fn query_balance(executor: &EvmExecutor, addr: &Address) -> U256 {
    match executor.query("eth_getBalance", addr.as_slice()) {
        Ok(resp) if resp.data.len() == 32 => U256::from_be_slice(&resp.data),
        _ => U256::ZERO,
    }
}

fn query_nonce(executor: &EvmExecutor, addr: &Address) -> u64 {
    match executor.query("eth_getTransactionCount", addr.as_slice()) {
        Ok(resp) if resp.data.len() == 8 => u64::from_be_bytes(resp.data.try_into().unwrap()),
        _ => 0,
    }
}

fn query_block_number(executor: &EvmExecutor) -> u64 {
    match executor.query("eth_blockNumber", &[]) {
        Ok(resp) if resp.data.len() == 8 => u64::from_be_bytes(resp.data.try_into().unwrap()),
        _ => 0,
    }
}

fn query_code(executor: &EvmExecutor, addr: &Address) -> Vec<u8> {
    match executor.query("eth_getCode", addr.as_slice()) {
        Ok(resp) => resp.data,
        _ => vec![],
    }
}

fn query_storage(executor: &EvmExecutor, addr: &Address, slot: &U256) -> U256 {
    let mut data = [0u8; 52];
    data[..20].copy_from_slice(addr.as_slice());
    data[20..].copy_from_slice(&slot.to_be_bytes::<32>());
    match executor.query("eth_getStorageAt", &data) {
        Ok(resp) if resp.data.len() == 32 => U256::from_be_slice(&resp.data),
        _ => U256::ZERO,
    }
}

fn dispatch(state: &EvmRpcState, req: &JsonRpcRequest) -> JsonRpcResponse {
    match req.method.as_str() {
        // --- Chain ---
        "eth_chainId" => JsonRpcResponse::ok(req.id.clone(), to_hex_u64(state.chain_id)),
        "net_version" => JsonRpcResponse::ok(
            req.id.clone(),
            serde_json::Value::String(state.chain_id.to_string()),
        ),
        "web3_clientVersion" => JsonRpcResponse::ok(
            req.id.clone(),
            serde_json::Value::String("hotmint-evm/0.1.0".to_string()),
        ),

        // --- Block ---
        "eth_blockNumber" => {
            let height = query_block_number(&state.executor);
            JsonRpcResponse::ok(req.id.clone(), to_hex_u64(height))
        }

        // --- Account state ---
        "eth_getBalance" => {
            let (addr, _block) = match parse_addr_block(&req.params) {
                Ok(v) => v,
                Err(e) => return JsonRpcResponse::err(req.id.clone(), -32602, &e),
            };
            let result = query_balance(&state.executor, &addr);
            JsonRpcResponse::ok(req.id.clone(), to_hex_u256(result))
        }
        "eth_getTransactionCount" => {
            let (addr, _block) = match parse_addr_block(&req.params) {
                Ok(v) => v,
                Err(e) => return JsonRpcResponse::err(req.id.clone(), -32602, &e),
            };
            let result = query_nonce(&state.executor, &addr);
            JsonRpcResponse::ok(req.id.clone(), to_hex_u64(result))
        }
        "eth_getCode" => {
            let (addr, _block) = match parse_addr_block(&req.params) {
                Ok(v) => v,
                Err(e) => return JsonRpcResponse::err(req.id.clone(), -32602, &e),
            };
            let code = query_code(&state.executor, &addr);
            JsonRpcResponse::ok(
                req.id.clone(),
                serde_json::Value::String(format!("0x{}", hex::encode(code))),
            )
        }
        "eth_getStorageAt" => {
            let arr = match req.params.as_array() {
                Some(a) if a.len() >= 2 => a,
                _ => {
                    return JsonRpcResponse::err(
                        req.id.clone(),
                        -32602,
                        "expected [addr, slot, block]",
                    );
                }
            };
            let addr = match parse_address(&arr[0]) {
                Ok(a) => a,
                Err(e) => return JsonRpcResponse::err(req.id.clone(), -32602, &e),
            };
            let slot = match parse_u256(&arr[1]) {
                Ok(s) => s,
                Err(e) => return JsonRpcResponse::err(req.id.clone(), -32602, &e),
            };
            let val = query_storage(&state.executor, &addr, &slot);
            JsonRpcResponse::ok(req.id.clone(), to_hex_u256(val))
        }

        // --- Gas ---
        "eth_gasPrice" => {
            let config = state.executor.config();
            JsonRpcResponse::ok(req.id.clone(), to_hex_u64(config.base_fee_per_gas))
        }
        "eth_estimateGas" => JsonRpcResponse::ok(req.id.clone(), to_hex_u64(21_000)),
        "eth_maxPriorityFeePerGas" => {
            JsonRpcResponse::ok(req.id.clone(), to_hex_u64(1_000_000_000))
        }

        // --- Transaction ---
        "eth_sendRawTransaction" => {
            let raw = match req.params.as_array().and_then(|p| p.first()) {
                Some(v) => match v.as_str() {
                    Some(s) => {
                        let s = s.strip_prefix("0x").unwrap_or(s);
                        match hex::decode(s) {
                            Ok(b) => b,
                            Err(e) => {
                                return JsonRpcResponse::err(
                                    req.id.clone(),
                                    -32602,
                                    &format!("invalid hex: {e}"),
                                );
                            }
                        }
                    }
                    None => {
                        return JsonRpcResponse::err(req.id.clone(), -32602, "expected hex string");
                    }
                },
                None => return JsonRpcResponse::err(req.id.clone(), -32602, "missing params"),
            };

            match state.executor.submit_raw_tx(&raw) {
                Ok(hash) => JsonRpcResponse::ok(
                    req.id.clone(),
                    serde_json::Value::String(format!("0x{}", hex::encode(hash))),
                ),
                Err(e) => JsonRpcResponse::err(req.id.clone(), -32000, &e),
            }
        }

        // --- Block stubs (for wallet compatibility) ---
        "eth_getBlockByNumber" => {
            let config = state.executor.config();
            JsonRpcResponse::ok(
                req.id.clone(),
                serde_json::json!({
                    "number": "0x0",
                    "hash": format!("0x{}", hex::encode(B256::ZERO)),
                    "parentHash": format!("0x{}", hex::encode(B256::ZERO)),
                    "timestamp": "0x0",
                    "gasLimit": to_hex_u64(config.block_gas_limit),
                    "gasUsed": "0x0",
                    "baseFeePerGas": to_hex_u64(config.base_fee_per_gas),
                    "miner": format!("0x{}", hex::encode(Address::ZERO)),
                    "transactions": [],
                }),
            )
        }
        "eth_getBlockByHash" => JsonRpcResponse::ok(req.id.clone(), serde_json::Value::Null),
        "eth_getTransactionByHash" => JsonRpcResponse::ok(req.id.clone(), serde_json::Value::Null),
        "eth_getTransactionReceipt" => JsonRpcResponse::ok(req.id.clone(), serde_json::Value::Null),
        "eth_getLogs" => JsonRpcResponse::ok(req.id.clone(), serde_json::json!([])),
        "eth_call" => {
            JsonRpcResponse::ok(req.id.clone(), serde_json::Value::String("0x".to_string()))
        }
        "eth_feeHistory" => {
            let base_fee = state.executor.config().base_fee_per_gas;
            JsonRpcResponse::ok(
                req.id.clone(),
                serde_json::json!({
                    "oldestBlock": "0x0",
                    "baseFeePerGas": [to_hex_u64(base_fee), to_hex_u64(base_fee)],
                    "gasUsedRatio": [0.0],
                    "reward": [[to_hex_u64(1_000_000_000)]]
                }),
            )
        }
        "eth_syncing" => JsonRpcResponse::ok(req.id.clone(), serde_json::Value::Bool(false)),
        "eth_accounts" => JsonRpcResponse::ok(req.id.clone(), serde_json::json!([])),

        _ => {
            warn!(method = %req.method, "unknown RPC method");
            JsonRpcResponse::err(
                req.id.clone(),
                -32601,
                &format!("method not found: {}", req.method),
            )
        }
    }
}

// --- Hex encoding helpers ---

fn to_hex_u64(v: u64) -> serde_json::Value {
    serde_json::Value::String(format!("0x{v:x}"))
}

fn to_hex_u256(v: U256) -> serde_json::Value {
    serde_json::Value::String(format!("0x{v:x}"))
}

fn parse_address(v: &serde_json::Value) -> Result<Address, String> {
    let s = v.as_str().ok_or("expected string")?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).map_err(|e| format!("invalid hex: {e}"))?;
    if bytes.len() != 20 {
        return Err(format!("address must be 20 bytes, got {}", bytes.len()));
    }
    Ok(Address::from_slice(&bytes))
}

fn parse_addr_block(params: &serde_json::Value) -> Result<(Address, String), String> {
    let arr = params.as_array().ok_or("expected array params")?;
    if arr.is_empty() {
        return Err("missing address parameter".to_string());
    }
    let addr = parse_address(&arr[0])?;
    let block = arr
        .get(1)
        .and_then(|v| v.as_str())
        .unwrap_or("latest")
        .to_string();
    Ok((addr, block))
}

fn parse_u256(v: &serde_json::Value) -> Result<U256, String> {
    let s = v.as_str().ok_or("expected hex string")?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(s, 16).map_err(|e| format!("invalid U256: {e}"))
}
