use ruc::*;

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use crate::types::{
    BlockInfo, BlockResultsInfo, CommitQcInfo, EpochInfo, EventAttributeInfo, EventInfo,
    HeaderInfo, QueryResponseInfo, RpcRequest, RpcResponse, StatusInfo, TxInfo, TxResult,
    ValidatorInfoResponse, VerifyHeaderResult,
};
use hotmint_consensus::application::Application;
use hotmint_consensus::commit::decode_payload;
use hotmint_consensus::store::BlockStore;
use hotmint_mempool::Mempool;
use hotmint_network::service::PeerStatus;
use hotmint_types::{BlockHash, Height};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio::time::{Duration, Instant, timeout};
use tracing::{info, warn};

const MAX_RPC_CONNECTIONS: usize = 256;
const RPC_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum bytes per RPC line. Prevents OOM from clients sending huge data without newlines.
const MAX_LINE_BYTES: usize = 1_048_576;
/// Maximum submit_tx calls per second per IP address.
pub(crate) const TX_RATE_LIMIT_PER_SEC: u32 = 100;
/// How often to prune stale per-IP rate limiter entries.
const IP_LIMITER_PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// Named consensus status shared via watch channel.
#[derive(Debug, Clone, Copy)]
pub struct ConsensusStatus {
    pub current_view: u64,
    pub last_committed_height: u64,
    pub epoch_number: u64,
    pub validator_count: usize,
    pub epoch_start_view: u64,
}

impl ConsensusStatus {
    pub fn new(
        current_view: u64,
        last_committed_height: u64,
        epoch_number: u64,
        validator_count: usize,
        epoch_start_view: u64,
    ) -> Self {
        Self {
            current_view,
            last_committed_height,
            epoch_number,
            validator_count,
            epoch_start_view,
        }
    }
}

/// Shared state accessible by the RPC server
pub struct RpcState {
    pub validator_id: u64,
    pub mempool: Arc<Mempool>,
    pub status_rx: watch::Receiver<ConsensusStatus>,
    /// Shared block store for block queries
    pub store: Arc<parking_lot::RwLock<Box<dyn BlockStore>>>,
    /// Peer info channel
    pub peer_info_rx: watch::Receiver<Vec<PeerStatus>>,
    /// Live validator set for get_validators
    pub validator_set_rx: watch::Receiver<Vec<ValidatorInfoResponse>>,
    /// Application reference for tx validation (optional for backward compatibility).
    pub app: Option<Arc<dyn Application>>,
    /// Optional sender to gossip accepted transactions to peers.
    pub tx_gossip: Option<mpsc::Sender<Vec<u8>>>,
    /// Chain ID hash for light client verification.
    pub chain_id_hash: [u8; 32],
}

/// Simple JSON-RPC server over TCP (one JSON object per line)
pub struct RpcServer {
    state: Arc<RpcState>,
    listener: TcpListener,
}

impl RpcServer {
    pub async fn bind(addr: &str, state: RpcState) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .c(d!("failed to bind RPC server"))?;
        info!(addr = addr, "RPC server listening");
        Ok(Self {
            state: Arc::new(state),
            listener,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().expect("listener has local addr")
    }

    pub async fn run(self) {
        let semaphore = Arc::new(Semaphore::new(MAX_RPC_CONNECTIONS));
        let ip_limiter = Arc::new(Mutex::new(PerIpRateLimiter::new()));
        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    let permit = match semaphore.clone().try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            warn!("RPC connection limit reached, rejecting");
                            drop(stream);
                            continue;
                        }
                    };
                    let state = self.state.clone();
                    let ip_limiter = ip_limiter.clone();
                    let peer_ip = addr.ip();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let (reader, mut writer) = stream.into_split();
                        let mut reader = BufReader::with_capacity(65_536, reader);
                        loop {
                            let line = match timeout(
                                RPC_READ_TIMEOUT,
                                read_line_limited(&mut reader, MAX_LINE_BYTES),
                            )
                            .await
                            {
                                Ok(Ok(Some(line))) => line,
                                Ok(Err(e)) => {
                                    warn!(error = %e, "RPC read error (line too long?)");
                                    break;
                                }
                                _ => break, // EOF or timeout
                            };
                            let response =
                                handle_request(&state, &line, &ip_limiter, peer_ip).await;
                            let mut json = serde_json::to_string(&response).unwrap_or_default();
                            json.push('\n');
                            if writer.write_all(json.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                Err(e) => {
                    warn!(error = %e, "failed to accept connection");
                }
            }
        }
    }
}

/// Token-bucket rate limiter for submit_tx.
pub struct TxRateLimiter {
    tokens: u32,
    max_tokens: u32,
    last_refill: Instant,
}

impl TxRateLimiter {
    pub(crate) fn new(rate_per_sec: u32) -> Self {
        Self {
            tokens: rate_per_sec,
            max_tokens: rate_per_sec,
            last_refill: Instant::now(),
        }
    }

    pub(crate) fn allow(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        if elapsed >= Duration::from_secs(1) {
            self.tokens = self.max_tokens;
            self.last_refill = now;
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

/// Per-IP rate limiter that tracks token buckets per source IP.
///
/// Maximum number of tracked IPs in the per-IP rate limiter.
const MAX_IP_LIMITER_ENTRIES: usize = 100_000;

/// Prevents a single IP from monopolising `submit_tx` even when opening
/// many TCP connections or HTTP requests.
pub struct PerIpRateLimiter {
    buckets: HashMap<IpAddr, TxRateLimiter>,
    last_prune: Instant,
}

impl Default for PerIpRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl PerIpRateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            last_prune: Instant::now(),
        }
    }

    /// Check whether `ip` is allowed to submit a transaction.
    pub fn allow(&mut self, ip: IpAddr) -> bool {
        self.maybe_prune();
        // Cap the number of tracked IPs to prevent memory exhaustion.
        if !self.buckets.contains_key(&ip) && self.buckets.len() >= MAX_IP_LIMITER_ENTRIES {
            return false;
        }
        let bucket = self
            .buckets
            .entry(ip)
            .or_insert_with(|| TxRateLimiter::new(TX_RATE_LIMIT_PER_SEC));
        bucket.allow()
    }

    /// Remove entries that have not been touched for a while to avoid unbounded growth.
    fn maybe_prune(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_prune) < IP_LIMITER_PRUNE_INTERVAL {
            return;
        }
        self.last_prune = now;
        // Remove buckets that are fully refilled (idle for ≥1 s)
        self.buckets
            .retain(|_, v| now.duration_since(v.last_refill) < Duration::from_secs(30));
    }
}

pub(crate) async fn handle_request(
    state: &RpcState,
    line: &str,
    ip_limiter: &Mutex<PerIpRateLimiter>,
    peer_ip: IpAddr,
) -> RpcResponse {
    let req: RpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return RpcResponse::err(0, -32700, format!("parse error: {e}"));
        }
    };

    match req.method.as_str() {
        "status" => {
            let s = *state.status_rx.borrow();
            let info = StatusInfo {
                validator_id: state.validator_id,
                current_view: s.current_view,
                last_committed_height: s.last_committed_height,
                epoch: s.epoch_number,
                validator_count: s.validator_count,
                mempool_size: state.mempool.size().await,
            };
            json_ok(req.id, &info)
        }

        "submit_tx" => {
            // Per-IP rate limiting (C-2: prevents bypass via multiple connections)
            {
                let mut limiter = ip_limiter.lock().await;
                if !limiter.allow(peer_ip) {
                    return RpcResponse::err(
                        req.id,
                        -32000,
                        "rate limit exceeded for submit_tx".to_string(),
                    );
                }
            }
            let Some(tx_hex) = req.params.as_str() else {
                return RpcResponse::err(req.id, -32602, "params must be a hex string".to_string());
            };
            if tx_hex.is_empty() {
                return RpcResponse::err(req.id, -32602, "empty transaction".to_string());
            }
            let tx_bytes = match hex_decode(tx_hex) {
                Some(b) if !b.is_empty() => b,
                _ => {
                    return RpcResponse::err(req.id, -32602, "invalid hex".to_string());
                }
            };
            // Validate via Application if available
            let (priority, gas_wanted) = if let Some(ref app) = state.app {
                let result = app.validate_tx(&tx_bytes, None);
                if !result.valid {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "transaction validation failed".to_string(),
                    );
                }
                (result.priority, result.gas_wanted)
            } else {
                (0, 0)
            };
            let accepted = state
                .mempool
                .add_tx_with_gas(tx_bytes.clone(), priority, gas_wanted)
                .await;
            // Gossip accepted transactions to peers.
            if accepted && let Some(ref gossip) = state.tx_gossip {
                let _ = gossip.try_send(tx_bytes);
            }
            json_ok(req.id, &TxResult { accepted })
        }

        "get_block" => {
            let height = match req.params.get("height").and_then(|v| v.as_u64()) {
                Some(h) => h,
                None => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "missing or invalid 'height' parameter".to_string(),
                    );
                }
            };
            let store = state.store.read();
            match store.get_block_by_height(Height(height)) {
                Some(block) => json_ok(req.id, &block_to_info(&block)),
                None => RpcResponse::err(
                    req.id,
                    -32602,
                    format!("block at height {height} not found"),
                ),
            }
        }

        "get_block_by_hash" => {
            let hash_hex = req.params.as_str().unwrap_or_default();
            match hex_to_block_hash(hash_hex) {
                Some(hash) => {
                    let store = state.store.read();
                    match store.get_block(&hash) {
                        Some(block) => json_ok(req.id, &block_to_info(&block)),
                        None => RpcResponse::err(req.id, -32602, "block not found".to_string()),
                    }
                }
                None => RpcResponse::err(req.id, -32602, "invalid hash hex".to_string()),
            }
        }

        "get_validators" => {
            let validators = state.validator_set_rx.borrow().clone();
            json_ok(req.id, &validators)
        }

        "get_epoch" => {
            let s = *state.status_rx.borrow();
            let info = EpochInfo {
                number: s.epoch_number,
                start_view: s.epoch_start_view,
                validator_count: s.validator_count,
            };
            json_ok(req.id, &info)
        }

        "get_peers" => {
            let peers = state.peer_info_rx.borrow().clone();
            json_ok(req.id, &peers)
        }

        "get_header" => {
            let height = match req.params.get("height").and_then(|v| v.as_u64()) {
                Some(h) => h,
                None => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "missing or invalid 'height' parameter".to_string(),
                    );
                }
            };
            let store = state.store.read();
            match store.get_block_by_height(Height(height)) {
                Some(block) => {
                    let info = HeaderInfo {
                        height: block.height.as_u64(),
                        hash: hex_encode(&block.hash.0),
                        parent_hash: hex_encode(&block.parent_hash.0),
                        view: block.view.as_u64(),
                        proposer: block.proposer.0,
                        app_hash: hex_encode(&block.app_hash.0),
                    };
                    json_ok(req.id, &info)
                }
                None => RpcResponse::err(
                    req.id,
                    -32602,
                    format!("block at height {height} not found"),
                ),
            }
        }

        "get_commit_qc" => {
            let height = match req.params.get("height").and_then(|v| v.as_u64()) {
                Some(h) => h,
                None => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "missing or invalid 'height' parameter".to_string(),
                    );
                }
            };
            let store = state.store.read();
            match store.get_commit_qc(Height(height)) {
                Some(qc) => {
                    let info = CommitQcInfo {
                        block_hash: hex_encode(&qc.block_hash.0),
                        view: qc.view.as_u64(),
                        signer_count: qc.aggregate_signature.count(),
                        epoch: qc.epoch.as_u64(),
                    };
                    json_ok(req.id, &info)
                }
                None => RpcResponse::err(
                    req.id,
                    -32602,
                    format!("commit QC at height {height} not found"),
                ),
            }
        }

        "get_tx" => {
            let hash_hex = match req.params.as_str() {
                Some(h) if !h.is_empty() => h,
                _ => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "params must be a hex-encoded tx hash".to_string(),
                    );
                }
            };
            let hash_bytes = match hex_decode(hash_hex) {
                Some(b) if b.len() == 32 => {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&b);
                    arr
                }
                _ => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "invalid tx hash (expected 32-byte hex)".to_string(),
                    );
                }
            };
            let store = state.store.read();
            match store.get_tx_location(&hash_bytes) {
                Some((height, index)) => match store.get_block_by_height(height) {
                    Some(block) => {
                        let txs = decode_payload(&block.payload);
                        match txs.get(index as usize) {
                            Some(tx_bytes) => {
                                let info = TxInfo {
                                    tx_hash: hash_hex.to_string(),
                                    height: height.as_u64(),
                                    index,
                                    data: hex_encode(tx_bytes),
                                };
                                json_ok(req.id, &info)
                            }
                            None => RpcResponse::err(
                                req.id,
                                -32602,
                                "tx index out of range in block".to_string(),
                            ),
                        }
                    }
                    None => RpcResponse::err(
                        req.id,
                        -32602,
                        format!("block at height {} not found", height.as_u64()),
                    ),
                },
                None => RpcResponse::err(req.id, -32602, "transaction not found".to_string()),
            }
        }

        "get_block_results" => {
            let height = match req.params.get("height").and_then(|v| v.as_u64()) {
                Some(h) => h,
                None => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "missing or invalid 'height' parameter".to_string(),
                    );
                }
            };
            let store = state.store.read();
            match store.get_block_results(Height(height)) {
                Some(results) => {
                    // Also compute tx hashes from the block payload.
                    let tx_hashes = if let Some(block) = store.get_block_by_height(Height(height)) {
                        decode_payload(&block.payload)
                            .iter()
                            .map(|tx| hex_encode(blake3::hash(tx).as_bytes()))
                            .collect()
                    } else {
                        vec![]
                    };
                    let info = BlockResultsInfo {
                        height,
                        tx_hashes,
                        events: results
                            .events
                            .iter()
                            .map(|e| EventInfo {
                                r#type: e.r#type.clone(),
                                attributes: e
                                    .attributes
                                    .iter()
                                    .map(|a| EventAttributeInfo {
                                        key: a.key.clone(),
                                        value: a.value.clone(),
                                    })
                                    .collect(),
                            })
                            .collect(),
                        app_hash: hex_encode(&results.app_hash.0),
                    };
                    json_ok(req.id, &info)
                }
                None => RpcResponse::err(
                    req.id,
                    -32602,
                    format!("block results at height {height} not found"),
                ),
            }
        }

        "query" => {
            let path = match req.params.get("path").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "missing 'path' parameter".to_string(),
                    );
                }
            };
            let data_hex = req
                .params
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let data = hex_decode(data_hex).unwrap_or_default();
            match &state.app {
                Some(app) => match app.query(path, &data) {
                    Ok(resp) => {
                        let info = QueryResponseInfo {
                            data: hex_encode(&resp.data),
                            proof: resp.proof.as_ref().map(|p| hex_encode(p)),
                            height: resp.height,
                        };
                        json_ok(req.id, &info)
                    }
                    Err(e) => RpcResponse::err(req.id, -32602, format!("query failed: {e}")),
                },
                None => RpcResponse::err(
                    req.id,
                    -32602,
                    "no application available for queries".to_string(),
                ),
            }
        }

        "verify_header" => {
            // Accepts { header: BlockHeader JSON, qc: CommitQC JSON }.
            // Uses the light client to verify the header.
            let header_val = match req.params.get("header") {
                Some(v) => v,
                None => {
                    return RpcResponse::err(
                        req.id,
                        -32602,
                        "missing 'header' parameter".to_string(),
                    );
                }
            };
            let qc_val = match req.params.get("qc") {
                Some(v) => v,
                None => {
                    return RpcResponse::err(req.id, -32602, "missing 'qc' parameter".to_string());
                }
            };
            let header: hotmint_light::BlockHeader =
                match serde_json::from_value(header_val.clone()) {
                    Ok(h) => h,
                    Err(e) => {
                        return RpcResponse::err(req.id, -32602, format!("invalid header: {e}"));
                    }
                };
            let qc: hotmint_types::QuorumCertificate = match serde_json::from_value(qc_val.clone())
            {
                Ok(q) => q,
                Err(e) => {
                    return RpcResponse::err(req.id, -32602, format!("invalid qc: {e}"));
                }
            };
            // Build a LightClient from the current validator set.
            let validators_info = state.validator_set_rx.borrow().clone();
            let validators: Vec<hotmint_types::ValidatorInfo> = validators_info
                .iter()
                .map(|v| hotmint_types::ValidatorInfo {
                    id: hotmint_types::ValidatorId(v.id),
                    public_key: {
                        let bytes = hex_decode(&v.public_key).unwrap_or_default();
                        let mut arr = [0u8; 32];
                        if bytes.len() == 32 {
                            arr.copy_from_slice(&bytes);
                        }
                        hotmint_types::PublicKey(arr.to_vec())
                    },
                    power: v.power,
                })
                .collect();
            let vs = hotmint_types::ValidatorSet::new(validators);
            let status = *state.status_rx.borrow();
            let lc = hotmint_light::LightClient::new(
                vs.clone(),
                hotmint_types::Height(status.last_committed_height),
                state.chain_id_hash,
            );
            let verifier = hotmint_crypto::Ed25519Verifier;
            match lc.verify_header(&header, &qc, &verifier) {
                Ok(()) => json_ok(
                    req.id,
                    &VerifyHeaderResult {
                        valid: true,
                        error: None,
                    },
                ),
                Err(e) => json_ok(
                    req.id,
                    &VerifyHeaderResult {
                        valid: false,
                        error: Some(e.to_string()),
                    },
                ),
            }
        }

        _ => RpcResponse::err(req.id, -32601, format!("unknown method: {}", req.method)),
    }
}

fn json_ok<T: serde::Serialize>(id: u64, val: &T) -> RpcResponse {
    match serde_json::to_value(val) {
        Ok(v) => RpcResponse::ok(id, v),
        Err(e) => RpcResponse::err(id, -32603, format!("serialization error: {e}")),
    }
}

fn block_to_info(block: &hotmint_types::Block) -> BlockInfo {
    BlockInfo {
        height: block.height.as_u64(),
        hash: hex_encode(&block.hash.0),
        parent_hash: hex_encode(&block.parent_hash.0),
        view: block.view.as_u64(),
        proposer: block.proposer.0,
        payload_size: block.payload.len(),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn hex_to_block_hash(s: &str) -> Option<BlockHash> {
    let bytes = hex_decode(s)?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(BlockHash(arr))
}

/// Read a line from `reader`, failing fast if it exceeds `max_bytes`.
///
/// Uses `fill_buf` + incremental scanning so memory allocation is bounded.
/// Returns `Ok(None)` on EOF, `Ok(Some(line))` on success, or an error
/// if the line exceeds the limit.
async fn read_line_limited<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
            };
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..pos]);
            reader.consume(pos + 1);
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let to_consume = available.len();
        buf.extend_from_slice(available);
        reader.consume(to_consume);
        if buf.len() > max_bytes {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
    }
}
