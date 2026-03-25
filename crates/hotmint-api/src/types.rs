use serde::{Deserialize, Serialize};

/// JSON-RPC request
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub method: String,
    pub params: serde_json::Value,
    pub id: u64,
}

/// JSON-RPC response
#[derive(Debug, Serialize, Deserialize)]
pub struct RpcResponse {
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
    pub id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcResponse {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn err(id: u64, code: i32, message: String) -> Self {
        Self {
            result: None,
            error: Some(RpcError { code, message }),
            id,
        }
    }
}

/// Consensus status info
#[derive(Debug, Clone, Serialize)]
pub struct StatusInfo {
    pub validator_id: u64,
    pub current_view: u64,
    pub last_committed_height: u64,
    pub epoch: u64,
    pub validator_count: usize,
    pub mempool_size: usize,
}

/// Block info returned by get_block / get_block_by_hash
#[derive(Debug, Serialize)]
pub struct BlockInfo {
    pub height: u64,
    pub hash: String,
    pub parent_hash: String,
    pub view: u64,
    pub proposer: u64,
    pub payload_size: usize,
}

/// Validator info returned by get_validators
#[derive(Debug, Clone, Serialize)]
pub struct ValidatorInfoResponse {
    pub id: u64,
    pub power: u64,
    pub public_key: String,
}

/// Epoch info returned by get_epoch
#[derive(Debug, Serialize)]
pub struct EpochInfo {
    pub number: u64,
    pub start_view: u64,
    pub validator_count: usize,
}

/// Transaction submission result
#[derive(Debug, Serialize)]
pub struct TxResult {
    pub accepted: bool,
}

/// Block header info returned by get_header (lightweight, no payload)
#[derive(Debug, Serialize)]
pub struct HeaderInfo {
    pub height: u64,
    pub hash: String,
    pub parent_hash: String,
    pub view: u64,
    pub proposer: u64,
    pub app_hash: String,
}

/// Commit QC info returned by get_commit_qc
#[derive(Debug, Serialize)]
pub struct CommitQcInfo {
    pub block_hash: String,
    pub view: u64,
    pub signer_count: usize,
    pub epoch: u64,
}

/// Response for the `get_tx` RPC method.
#[derive(Serialize)]
pub struct TxInfo {
    pub tx_hash: String,
    pub height: u64,
    pub index: u32,
    /// Hex-encoded transaction bytes.
    pub data: String,
}

/// Response for the `query` RPC method.
#[derive(Serialize)]
pub struct QueryResponseInfo {
    /// Hex-encoded result data.
    pub data: String,
    /// Hex-encoded Merkle proof (if provided by the application).
    pub proof: Option<String>,
    pub height: u64,
}

/// Response for the `verify_header` RPC method.
#[derive(Serialize)]
pub struct VerifyHeaderResult {
    pub valid: bool,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct BlockResultsInfo {
    pub height: u64,
    pub tx_hashes: Vec<String>,
    pub events: Vec<EventInfo>,
    pub app_hash: String,
}

/// A single application event in the block results response.
#[derive(Serialize)]
pub struct EventInfo {
    pub r#type: String,
    pub attributes: Vec<EventAttributeInfo>,
}

#[derive(Serialize)]
pub struct EventAttributeInfo {
    pub key: String,
    pub value: String,
}
