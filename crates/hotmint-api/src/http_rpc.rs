use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use tokio::sync::Mutex;

use crate::rpc::{PerIpRateLimiter, RpcState, handle_request};
use crate::types::RpcResponse;

/// Events broadcast to WebSocket subscribers.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum ChainEvent {
    NewBlock {
        height: u64,
        hash: String,
        view: u64,
        proposer: u64,
        timestamp: u64,
    },
    /// Emitted when a transaction is included in a committed block.
    TxCommitted { tx_hash: String, height: u64 },
    /// Emitted when a new epoch begins (validator set change).
    EpochChange {
        epoch: u64,
        start_view: u64,
        validator_count: usize,
    },
}

/// Shared state for the HTTP RPC server.
pub struct HttpRpcState {
    pub rpc: Arc<RpcState>,
    pub event_tx: broadcast::Sender<ChainEvent>,
    /// Per-IP rate limiter for submit_tx (C-2: prevents bypass via multiple connections).
    pub ip_limiter: Mutex<PerIpRateLimiter>,
    /// Current number of active WebSocket connections.
    ws_connection_count: std::sync::atomic::AtomicUsize,
}

/// HTTP JSON-RPC server (runs alongside the existing TCP RPC server).
pub struct HttpRpcServer {
    state: Arc<HttpRpcState>,
    addr: SocketAddr,
}

impl HttpRpcServer {
    /// Create a new HTTP RPC server.
    ///
    /// `event_capacity` controls the broadcast channel buffer size for WebSocket events.
    pub fn new(addr: SocketAddr, rpc: Arc<RpcState>, event_capacity: usize) -> Self {
        let (event_tx, _) = broadcast::channel(event_capacity);
        Self {
            state: Arc::new(HttpRpcState {
                rpc,
                event_tx,
                ip_limiter: Mutex::new(PerIpRateLimiter::new()),
                ws_connection_count: std::sync::atomic::AtomicUsize::new(0),
            }),
            addr,
        }
    }

    /// Get a `broadcast::Sender` so the node can publish chain events.
    pub fn event_sender(&self) -> broadcast::Sender<ChainEvent> {
        self.state.event_tx.clone()
    }

    /// Run the HTTP server (blocks until shutdown).
    pub async fn run(self) {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        let app = Router::new()
            .route("/", post(json_rpc_handler))
            .route("/ws", get(ws_upgrade_handler))
            .layer(cors)
            .with_state(self.state.clone());

        let listener = match tokio::net::TcpListener::bind(self.addr).await {
            Ok(l) => l,
            Err(e) => {
                warn!(addr = %self.addr, error = %e, "HTTP RPC server failed to bind");
                return;
            }
        };

        let local_addr = listener.local_addr().expect("listener has local addr");
        info!(addr = %local_addr, "HTTP RPC server listening");

        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            warn!(error = %e, "HTTP RPC server exited with error");
        }
    }
}

/// POST / handler: parse JSON-RPC request body, dispatch, return JSON response.
async fn json_rpc_handler(
    State(state): State<Arc<HttpRpcState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: String,
) -> impl IntoResponse {
    // C-2: Per-IP rate limiting for submit_tx.
    let response: RpcResponse =
        handle_request(&state.rpc, &body, &state.ip_limiter, addr.ip()).await;

    axum::Json(response)
}

/// Maximum concurrent WebSocket connections to prevent resource exhaustion.
const MAX_WS_CONNECTIONS: usize = 1024;

/// GET /ws handler: upgrade to WebSocket and stream chain events.
async fn ws_upgrade_handler(
    State(state): State<Arc<HttpRpcState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let current = state
        .ws_connection_count
        .load(std::sync::atomic::Ordering::Relaxed);
    if current >= MAX_WS_CONNECTIONS {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "too many WebSocket connections",
        )
            .into_response();
    }
    ws.on_upgrade(move |socket| handle_ws(socket, state))
        .into_response()
}

/// Client-sent subscription filter for WebSocket events.
///
/// The client can send a JSON message to control which events are forwarded.
/// If no filter is sent, all events are forwarded.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SubscribeFilter {
    /// Event types to receive (e.g. ["NewBlock", "TxCommitted", "EpochChange"]).
    /// If empty or absent, all event types are forwarded.
    #[serde(default)]
    pub event_types: Vec<String>,
    /// Only forward events at or above this height (for NewBlock / TxCommitted).
    #[serde(default)]
    pub min_height: Option<u64>,
    /// Only forward events at or below this height.
    #[serde(default)]
    pub max_height: Option<u64>,
    /// Only forward TxCommitted events matching this tx hash (hex).
    #[serde(default)]
    pub tx_hash: Option<String>,
}

impl SubscribeFilter {
    fn matches(&self, event: &ChainEvent) -> bool {
        // Check event type filter.
        if !self.event_types.is_empty() {
            let event_type = match event {
                ChainEvent::NewBlock { .. } => "NewBlock",
                ChainEvent::TxCommitted { .. } => "TxCommitted",
                ChainEvent::EpochChange { .. } => "EpochChange",
            };
            if !self.event_types.iter().any(|t| t == event_type) {
                return false;
            }
        }
        // Check height range.
        let height = match event {
            ChainEvent::NewBlock { height, .. } | ChainEvent::TxCommitted { height, .. } => {
                Some(*height)
            }
            _ => None,
        };
        if let Some(h) = height {
            if let Some(min) = self.min_height {
                if h < min {
                    return false;
                }
            }
            if let Some(max) = self.max_height {
                if h > max {
                    return false;
                }
            }
        }
        // Check tx hash filter. When set, only TxCommitted events matching
        // the hash pass through; all other event types are excluded.
        if let Some(ref filter_hash) = self.tx_hash {
            match event {
                ChainEvent::TxCommitted { tx_hash, .. } => {
                    if tx_hash != filter_hash {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

/// WebSocket connection handler: subscribe to chain events and forward them.
async fn handle_ws(mut socket: WebSocket, state: Arc<HttpRpcState>) {
    state
        .ws_connection_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut rx = state.event_tx.subscribe();
    let mut filter = SubscribeFilter::default();

    loop {
        tokio::select! {
            // Forward broadcast events to the client
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        if !filter.matches(&ev) {
                            continue;
                        }
                        let json = match serde_json::to_string(&ev) {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "failed to serialize chain event");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "WebSocket client lagged, some events dropped");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            // Listen for client messages (subscription filters, close frames, pings)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Try to parse as a subscribe filter.
                        if let Ok(f) = serde_json::from_str::<SubscribeFilter>(&text) {
                            filter = f;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    state
        .ws_connection_count
        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}
