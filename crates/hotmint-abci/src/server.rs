use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hotmint_consensus::application::AppInfo;
use hotmint_consensus::liveness::OfflineEvidence;
use tokio::net::UnixListener;

use hotmint_types::Block;
use hotmint_types::BlockHash;
use hotmint_types::context::{OwnedBlockContext, TxContext};
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::sync::{ChunkApplyResult, SnapshotInfo, SnapshotOfferResult};
use hotmint_types::validator::ValidatorId;
use hotmint_types::validator_update::EndBlockResponse;

use crate::protocol::{self, Request, Response};

/// Owned-data callback interface for applications running in a separate process.
///
/// This is the cross-process counterpart of `hotmint_consensus::Application`.
/// All parameters are owned so they can be deserialized from the wire.
pub trait ApplicationHandler: Send + Sync {
    fn info(&self) -> AppInfo {
        AppInfo::default()
    }

    fn init_chain(&self, app_state: Vec<u8>) -> Result<BlockHash, String> {
        let _ = app_state;
        Ok(BlockHash::GENESIS)
    }

    fn create_payload(&self, ctx: OwnedBlockContext) -> Vec<u8> {
        let _ = ctx;
        vec![]
    }

    fn validate_block(&self, block: Block, ctx: OwnedBlockContext) -> bool {
        let _ = (block, ctx);
        true
    }

    fn validate_tx(&self, tx: Vec<u8>, ctx: Option<TxContext>) -> (bool, u64, u64) {
        let _ = (tx, ctx);
        (true, 0, 0)
    }

    fn execute_block(
        &self,
        txs: Vec<Vec<u8>>,
        ctx: OwnedBlockContext,
    ) -> Result<EndBlockResponse, String>;

    fn on_commit(&self, block: Block, ctx: OwnedBlockContext) -> Result<(), String> {
        let _ = (block, ctx);
        Ok(())
    }

    fn on_evidence(&self, proof: EquivocationProof) -> Result<(), String> {
        let _ = proof;
        Ok(())
    }

    fn on_offline_validators(&self, offline: Vec<OfflineEvidence>) -> Result<(), String> {
        let _ = offline;
        Ok(())
    }

    fn extend_vote(&self, block: Block, ctx: OwnedBlockContext) -> Option<Vec<u8>> {
        let _ = (block, ctx);
        None
    }

    fn verify_vote_extension(
        &self,
        extension: Vec<u8>,
        block_hash: BlockHash,
        validator: ValidatorId,
    ) -> bool {
        let _ = (extension, block_hash, validator);
        true
    }

    fn query(&self, path: String, data: Vec<u8>) -> Result<hotmint_types::QueryResponse, String> {
        let _ = (path, data);
        Ok(hotmint_types::QueryResponse::default())
    }

    fn list_snapshots(&self) -> Vec<SnapshotInfo> {
        vec![]
    }

    fn load_snapshot_chunk(&self, height: hotmint_types::Height, chunk_index: u32) -> Vec<u8> {
        let _ = (height, chunk_index);
        vec![]
    }

    fn offer_snapshot(&self, snapshot: SnapshotInfo) -> SnapshotOfferResult {
        let _ = snapshot;
        SnapshotOfferResult::Reject
    }

    fn apply_snapshot_chunk(&self, chunk: Vec<u8>, chunk_index: u32) -> ChunkApplyResult {
        let _ = (chunk, chunk_index);
        ChunkApplyResult::Abort
    }

    fn tracks_app_hash(&self) -> bool {
        true
    }
}

/// IPC server that listens on a Unix domain socket and dispatches incoming
/// requests to an [`ApplicationHandler`].
pub struct IpcApplicationServer<H> {
    socket_path: PathBuf,
    handler: Arc<H>,
}

impl<H> Drop for IpcApplicationServer<H> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl<H: ApplicationHandler + 'static> IpcApplicationServer<H> {
    pub fn new(socket_path: impl AsRef<Path>, handler: H) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_path_buf(),
            handler: Arc::new(handler),
        }
    }

    /// Run the server, accepting connections and processing requests.
    ///
    /// Each client connection is handled in its own task so health checks,
    /// sync clients, consensus callbacks, and bounded query clients cannot
    /// block each other at the accept loop.
    pub async fn run(&self) -> io::Result<()> {
        // Remove stale socket file if present.
        let _ = fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)?;
        tracing::info!(path = %self.socket_path.display(), "IPC server listening");

        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(error = %e, "IPC accept failed, continuing");
                    continue;
                }
            };
            tracing::debug!("IPC client connected");
            let handler = Arc::clone(&self.handler);
            tokio::spawn(async move {
                Self::handle_connection(handler, stream).await;
            });
        }
    }

    async fn handle_connection(handler: Arc<H>, mut stream: tokio::net::UnixStream) {
        loop {
            let frame = match protocol::read_frame(&mut stream).await {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    tracing::debug!("IPC client disconnected");
                    break;
                }
                Err(e) => {
                    tracing::error!(%e, "read_frame error");
                    break;
                }
            };

            let req: Request = match protocol::decode_request(&frame) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(%e, "failed to decode request");
                    break;
                }
            };

            let resp = Self::dispatch(handler.as_ref(), req);
            let resp_bytes = protocol::encode_response(&resp);

            if let Err(e) = protocol::write_frame(&mut stream, &resp_bytes).await {
                tracing::error!(%e, "write_frame error");
                break;
            }
        }
    }

    fn dispatch(handler: &H, req: Request) -> Response {
        match req {
            Request::Info => Response::Info(handler.info()),
            Request::InitChain(app_state) => Response::InitChain(handler.init_chain(app_state)),
            Request::CreatePayload(ctx) => Response::CreatePayload(handler.create_payload(ctx)),
            Request::ValidateBlock { block, ctx } => {
                Response::ValidateBlock(handler.validate_block(block, ctx))
            }
            Request::ValidateTx { tx, ctx } => {
                let (ok, priority, gas_wanted) = handler.validate_tx(tx, ctx);
                Response::ValidateTx {
                    ok,
                    priority,
                    gas_wanted,
                }
            }
            Request::ExecuteBlock { txs, ctx } => {
                Response::ExecuteBlock(handler.execute_block(txs, ctx))
            }
            Request::OnCommit { block, ctx } => Response::OnCommit(handler.on_commit(block, ctx)),
            Request::OnEvidence(proof) => Response::OnEvidence(handler.on_evidence(proof)),
            Request::OnOfflineValidators(offline) => {
                Response::OnOfflineValidators(handler.on_offline_validators(offline))
            }
            Request::ExtendVote { block, ctx } => {
                Response::ExtendVote(handler.extend_vote(block, ctx))
            }
            Request::VerifyVoteExtension {
                extension,
                block_hash,
                validator,
            } => Response::VerifyVoteExtension(
                handler.verify_vote_extension(extension, block_hash, validator),
            ),
            Request::Query { path, data } => {
                Response::Query(handler.query(path, data).map_err(|e| e.to_string()))
            }
            Request::ListSnapshots => Response::ListSnapshots(handler.list_snapshots()),
            Request::LoadSnapshotChunk {
                height,
                chunk_index,
            } => Response::LoadSnapshotChunk(handler.load_snapshot_chunk(height, chunk_index)),
            Request::OfferSnapshot(snapshot) => {
                Response::OfferSnapshot(handler.offer_snapshot(snapshot))
            }
            Request::ApplySnapshotChunk { chunk, chunk_index } => {
                Response::ApplySnapshotChunk(handler.apply_snapshot_chunk(chunk, chunk_index))
            }
            Request::TracksAppHash => Response::TracksAppHash(handler.tracks_app_hash()),
        }
    }
}
