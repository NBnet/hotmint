use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::Serialize;
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
    },
    // Future: TxCommitted, EpochChange, etc.
}

/// Shared state for the HTTP RPC server.
pub struct HttpRpcState {
    pub rpc: Arc<RpcState>,
    pub event_tx: broadcast::Sender<ChainEvent>,
    /// Per-IP rate limiter for submit_tx (C-2: prevents bypass via multiple connections).
    pub ip_limiter: Mutex<PerIpRateLimiter>,
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

        if let Err(e) =
            axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await
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

/// GET /ws handler: upgrade to WebSocket and stream chain events.
async fn ws_upgrade_handler(
    State(state): State<Arc<HttpRpcState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// WebSocket connection handler: subscribe to chain events and forward them.
async fn handle_ws(mut socket: WebSocket, state: Arc<HttpRpcState>) {
    let mut rx = state.event_tx.subscribe();

    loop {
        tokio::select! {
            // Forward broadcast events to the client
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        let json = match serde_json::to_string(&ev) {
                            Ok(j) => j,
                            Err(e) => {
                                warn!(error = %e, "failed to serialize chain event");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            // Client disconnected
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
            // Listen for client messages (e.g. close frames, pings)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {} // ignore text/binary from client for now
                }
            }
        }
    }
}
