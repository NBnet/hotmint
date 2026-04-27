use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use ruc::*;

use hotmint_consensus::application::{AppInfo, Application, TxValidationResult};
use hotmint_consensus::liveness::OfflineEvidence;
use hotmint_types::Block;
use hotmint_types::BlockHash;
use hotmint_types::Height;
use hotmint_types::context::{BlockContext, OwnedBlockContext, TxContext};
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::sync::{ChunkApplyResult, SnapshotInfo, SnapshotOfferResult};
use hotmint_types::validator::ValidatorId;
use hotmint_types::validator_update::EndBlockResponse;

use crate::protocol::{self, Request, Response};

/// IPC client that implements [`Application`] by forwarding every call over a
/// Unix domain socket using length-prefixed protobuf frames.
pub struct IpcApplicationClient {
    socket_path: PathBuf,
    conn: Mutex<Option<UnixStream>>,
    tracks_app_hash_cache: Mutex<Option<bool>>,
}

impl IpcApplicationClient {
    pub fn new(socket_path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_path_buf(),
            conn: Mutex::new(None),
            tracks_app_hash_cache: Mutex::new(None),
        }
    }

    /// IPC read/write timeout.  Prevents the consensus engine from hanging
    /// indefinitely when the application process is deadlocked or unresponsive.
    const IPC_TIMEOUT: Duration = Duration::from_secs(5);

    /// Try to connect to the ABCI socket. Returns an error if unreachable.
    pub fn check_connection(&self) -> Result<()> {
        let stream = UnixStream::connect(&self.socket_path).c(d!("connect to ABCI socket"))?;
        Self::set_timeouts(&stream)?;
        drop(stream);
        Ok(())
    }

    fn set_timeouts(stream: &UnixStream) -> Result<()> {
        stream
            .set_read_timeout(Some(Self::IPC_TIMEOUT))
            .c(d!("set IPC read timeout"))?;
        stream
            .set_write_timeout(Some(Self::IPC_TIMEOUT))
            .c(d!("set IPC write timeout"))?;
        Ok(())
    }

    /// Send a request and wait for the response, lazily connecting on first use.
    fn call(&self, req: &Request) -> Result<Response> {
        let payload = protocol::encode_request(req);

        let mut guard = self.conn.lock().map_err(|e| eg!(e.to_string()))?;
        let resp_bytes = match self.call_once(&mut guard, &payload) {
            Ok(bytes) => bytes,
            Err(CallFailure::Write(err)) if err.bytes_written == 0 => {
                *guard = None;
                self.call_once(&mut guard, &payload)
                    .map_err(|failure| eg!(failure.into_error()))
                    .c(d!("retry request after pre-write IPC failure"))?
            }
            Err(CallFailure::Read(_)) if req.may_retry_after_write() => {
                *guard = None;
                self.call_once(&mut guard, &payload)
                    .map_err(|failure| eg!(failure.into_error()))
                    .c(d!("retry idempotent request after IPC read failure"))?
            }
            Err(failure) => {
                *guard = None;
                return Err(eg!(failure.into_error())).c(d!(
                    "IPC request failed; not retrying ambiguous post-write failure"
                ));
            }
        };
        let resp = protocol::decode_response(&resp_bytes)
            .map_err(|e| eg!(e.to_string()))
            .c(d!("decode response"))?;
        Ok(resp)
    }

    fn call_once(
        &self,
        guard: &mut Option<UnixStream>,
        payload: &[u8],
    ) -> std::result::Result<Vec<u8>, CallFailure> {
        if guard.is_none() {
            let stream = UnixStream::connect(&self.socket_path).map_err(CallFailure::Connect)?;
            Self::set_timeouts(&stream)
                .map_err(|e| CallFailure::Connect(io::Error::other(e.to_string())))?;
            *guard = Some(stream);
        }
        let stream = guard.as_mut().expect("IPC stream is connected");
        write_frame_sync(stream, payload).map_err(CallFailure::Write)?;
        read_frame_sync(stream).map_err(CallFailure::Read)
    }
}

enum CallFailure {
    Connect(io::Error),
    Write(FrameWriteError),
    Read(io::Error),
}

impl CallFailure {
    fn into_error(self) -> String {
        match self {
            Self::Connect(e) | Self::Read(e) => e.to_string(),
            Self::Write(e) => e.source.to_string(),
        }
    }
}

impl Request {
    fn may_retry_after_write(&self) -> bool {
        matches!(
            self,
            Request::Info
                | Request::ValidateBlock { .. }
                | Request::ValidateTx { .. }
                | Request::VerifyVoteExtension { .. }
                | Request::Query { .. }
                | Request::ListSnapshots
                | Request::LoadSnapshotChunk { .. }
                | Request::TracksAppHash
        )
    }
}

struct FrameWriteError {
    source: io::Error,
    bytes_written: usize,
}

fn write_frame_sync(
    w: &mut impl Write,
    payload: &[u8],
) -> std::result::Result<(), FrameWriteError> {
    const MAX_FRAME: usize = 64 * 1024 * 1024;
    if payload.len() > MAX_FRAME {
        return Err(FrameWriteError {
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame size {} exceeds max {MAX_FRAME}", payload.len()),
            ),
            bytes_written: 0,
        });
    }
    let len = payload.len() as u32;
    let mut bytes_written = 0;
    write_all_tracked(w, &len.to_le_bytes(), &mut bytes_written)?;
    write_all_tracked(w, payload, &mut bytes_written)?;
    w.flush().map_err(|source| FrameWriteError {
        source,
        bytes_written,
    })
}

fn write_all_tracked(
    w: &mut impl Write,
    mut buf: &[u8],
    bytes_written: &mut usize,
) -> std::result::Result<(), FrameWriteError> {
    while !buf.is_empty() {
        match w.write(buf) {
            Ok(0) => {
                return Err(FrameWriteError {
                    source: io::Error::new(io::ErrorKind::WriteZero, "failed to write frame"),
                    bytes_written: *bytes_written,
                });
            }
            Ok(n) => {
                *bytes_written += n;
                buf = &buf[n..];
            }
            Err(source) => {
                return Err(FrameWriteError {
                    source,
                    bytes_written: *bytes_written,
                });
            }
        }
    }
    Ok(())
}

fn read_frame_sync(r: &mut impl Read) -> io::Result<Vec<u8>> {
    const MAX_FRAME: usize = 64 * 1024 * 1024;
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame size {len} exceeds max {MAX_FRAME}"),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

impl Application for IpcApplicationClient {
    fn info(&self) -> AppInfo {
        let req = Request::Info;
        match self.call(&req) {
            Ok(Response::Info(info)) => info,
            Ok(other) => panic!("unexpected response for info: {other:?}"),
            Err(e) => panic!("IPC info call failed: {e}"),
        }
    }

    fn init_chain(&self, app_state: &[u8]) -> Result<BlockHash> {
        let req = Request::InitChain(app_state.to_vec());
        match self.call(&req)? {
            Response::InitChain(result) => result.map_err(|e| eg!(e)),
            other => Err(eg!(format!(
                "unexpected response for init_chain: {other:?}"
            ))),
        }
    }

    fn create_payload(&self, ctx: &BlockContext) -> Vec<u8> {
        let req = Request::CreatePayload(OwnedBlockContext::from(ctx));
        match self.call(&req) {
            Ok(Response::CreatePayload(payload)) => payload,
            Ok(other) => {
                tracing::error!(?other, "IPC_FAULT: unexpected response for create_payload");
                vec![]
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: create_payload call failed — proposing empty block");
                vec![]
            }
        }
    }

    fn validate_block(&self, block: &Block, ctx: &BlockContext) -> bool {
        let req = Request::ValidateBlock {
            block: block.clone(),
            ctx: OwnedBlockContext::from(ctx),
        };
        match self.call(&req) {
            Ok(Response::ValidateBlock(ok)) => ok,
            Ok(other) => {
                // Unexpected response variant indicates a protocol framing error.
                // Reset the connection so the next call reconnects and re-syncs the
                // framing, then reject the block so the view times out and recovers.
                *self.conn.lock().unwrap_or_else(|p| p.into_inner()) = None;
                tracing::error!(
                    ?other,
                    "IPC_FAULT: unexpected response for validate_block — rejecting block"
                );
                false
            }
            Err(e) => {
                // IPC fault: reset the cached connection so the next call reconnects,
                // then reject the block. The view will time out and the leader will
                // re-propose; once ABCI recovers the node resumes normal operation.
                *self.conn.lock().unwrap_or_else(|p| p.into_inner()) = None;
                tracing::error!(%e, "IPC_FAULT: validate_block call failed — rejecting block until ABCI recovers");
                false
            }
        }
    }

    fn validate_tx(&self, tx: &[u8], ctx: Option<&TxContext>) -> TxValidationResult {
        let req = Request::ValidateTx {
            tx: tx.to_vec(),
            ctx: ctx.cloned(),
        };
        match self.call(&req) {
            Ok(Response::ValidateTx {
                ok,
                priority,
                gas_wanted,
            }) => TxValidationResult {
                valid: ok,
                priority,
                gas_wanted,
            },
            Ok(other) => {
                tracing::error!(?other, "IPC_FAULT: unexpected response for validate_tx");
                TxValidationResult::reject()
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: validate_tx call failed — rejecting tx");
                TxValidationResult::reject()
            }
        }
    }

    fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
        let req = Request::ExecuteBlock {
            txs: txs.iter().map(|t| t.to_vec()).collect(),
            ctx: OwnedBlockContext::from(ctx),
        };
        match self.call(&req)? {
            Response::ExecuteBlock(result) => result.map_err(|e| eg!(e)),
            other => Err(eg!(format!(
                "unexpected response for execute_block: {other:?}"
            ))),
        }
    }

    fn on_commit(&self, block: &Block, ctx: &BlockContext) -> Result<()> {
        let req = Request::OnCommit {
            block: block.clone(),
            ctx: OwnedBlockContext::from(ctx),
        };
        match self.call(&req)? {
            Response::OnCommit(result) => result.map_err(|e| eg!(e)),
            other => Err(eg!(format!("unexpected response for on_commit: {other:?}"))),
        }
    }

    fn on_evidence(&self, proof: &EquivocationProof) -> Result<()> {
        let req = Request::OnEvidence(proof.clone());
        match self.call(&req)? {
            Response::OnEvidence(result) => result.map_err(|e| eg!(e)),
            other => Err(eg!(format!(
                "unexpected response for on_evidence: {other:?}"
            ))),
        }
    }

    fn on_offline_validators(&self, offline: &[OfflineEvidence]) -> Result<()> {
        let req = Request::OnOfflineValidators(offline.to_vec());
        match self.call(&req)? {
            Response::OnOfflineValidators(result) => result.map_err(|e| eg!(e)),
            other => Err(eg!(format!(
                "unexpected response for on_offline_validators: {other:?}"
            ))),
        }
    }

    fn extend_vote(&self, block: &Block, ctx: &BlockContext) -> Option<Vec<u8>> {
        let req = Request::ExtendVote {
            block: block.clone(),
            ctx: OwnedBlockContext::from(ctx),
        };
        match self.call(&req) {
            Ok(Response::ExtendVote(extension)) => extension,
            Ok(other) => {
                tracing::error!(?other, "IPC_FAULT: unexpected response for extend_vote");
                None
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: extend_vote call failed");
                None
            }
        }
    }

    fn verify_vote_extension(
        &self,
        extension: &[u8],
        block_hash: &BlockHash,
        validator: ValidatorId,
    ) -> bool {
        let req = Request::VerifyVoteExtension {
            extension: extension.to_vec(),
            block_hash: *block_hash,
            validator,
        };
        match self.call(&req) {
            Ok(Response::VerifyVoteExtension(ok)) => ok,
            Ok(other) => {
                tracing::error!(
                    ?other,
                    "IPC_FAULT: unexpected response for verify_vote_extension"
                );
                false
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: verify_vote_extension call failed");
                false
            }
        }
    }

    fn query(&self, path: &str, data: &[u8]) -> Result<hotmint_types::QueryResponse> {
        let req = Request::Query {
            path: path.to_string(),
            data: data.to_vec(),
        };
        match self.call(&req)? {
            Response::Query(result) => result.map_err(|e| eg!(e)),
            other => Err(eg!(format!("unexpected response for query: {other:?}"))),
        }
    }

    fn list_snapshots(&self) -> Vec<SnapshotInfo> {
        let req = Request::ListSnapshots;
        match self.call(&req) {
            Ok(Response::ListSnapshots(snapshots)) => snapshots,
            Ok(other) => {
                tracing::error!(?other, "IPC_FAULT: unexpected response for list_snapshots");
                vec![]
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: list_snapshots call failed");
                vec![]
            }
        }
    }

    fn load_snapshot_chunk(&self, height: Height, chunk_index: u32) -> Vec<u8> {
        let req = Request::LoadSnapshotChunk {
            height,
            chunk_index,
        };
        match self.call(&req) {
            Ok(Response::LoadSnapshotChunk(data)) => data,
            Ok(other) => {
                tracing::error!(
                    ?other,
                    "IPC_FAULT: unexpected response for load_snapshot_chunk"
                );
                vec![]
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: load_snapshot_chunk call failed");
                vec![]
            }
        }
    }

    fn offer_snapshot(&self, snapshot: &SnapshotInfo) -> SnapshotOfferResult {
        let req = Request::OfferSnapshot(snapshot.clone());
        match self.call(&req) {
            Ok(Response::OfferSnapshot(result)) => result,
            Ok(other) => {
                tracing::error!(?other, "IPC_FAULT: unexpected response for offer_snapshot");
                SnapshotOfferResult::Abort
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: offer_snapshot call failed");
                SnapshotOfferResult::Abort
            }
        }
    }

    fn apply_snapshot_chunk(&self, chunk: Vec<u8>, chunk_index: u32) -> ChunkApplyResult {
        let req = Request::ApplySnapshotChunk { chunk, chunk_index };
        match self.call(&req) {
            Ok(Response::ApplySnapshotChunk(result)) => result,
            Ok(other) => {
                tracing::error!(
                    ?other,
                    "IPC_FAULT: unexpected response for apply_snapshot_chunk"
                );
                ChunkApplyResult::Abort
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: apply_snapshot_chunk call failed");
                ChunkApplyResult::Abort
            }
        }
    }

    fn tracks_app_hash(&self) -> bool {
        if let Some(cached) = *self
            .tracks_app_hash_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            return cached;
        }

        let req = Request::TracksAppHash;
        let tracks = match self.call(&req) {
            Ok(Response::TracksAppHash(tracks)) => tracks,
            Ok(other) => {
                tracing::error!(?other, "IPC_FAULT: unexpected response for tracks_app_hash");
                true
            }
            Err(e) => {
                tracing::error!(%e, "IPC_FAULT: tracks_app_hash call failed");
                true
            }
        };
        *self
            .tracks_app_hash_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(tracks);
        tracks
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use hotmint_types::block::{BlockHash, Height};
    use hotmint_types::context::BlockContext;
    use hotmint_types::crypto::PublicKey;
    use hotmint_types::epoch::EpochNumber;
    use hotmint_types::validator::{ValidatorInfo, ValidatorSet};
    use hotmint_types::view::ViewNumber;

    use super::*;

    fn test_socket_path(name: &str) -> PathBuf {
        let dir = std::env::current_dir()
            .expect("current dir")
            .join(".copilot-tmp");
        std::fs::create_dir_all(&dir).expect("create test dir");
        dir.join(format!("{name}-{}.sock", std::process::id()))
    }

    fn make_validator_set() -> ValidatorSet {
        ValidatorSet::new(vec![ValidatorInfo {
            id: ValidatorId(0),
            public_key: PublicKey(vec![0; 32]),
            power: 1,
        }])
    }

    fn make_block_context(vs: &ValidatorSet) -> BlockContext<'_> {
        BlockContext {
            height: Height(1),
            view: ViewNumber(0),
            proposer: ValidatorId(0),
            epoch: EpochNumber(0),
            epoch_start_view: ViewNumber(0),
            validator_set: vs,
            timestamp: 0,
            vote_extensions: vec![],
        }
    }

    fn make_block() -> Block {
        Block {
            height: Height(1),
            parent_hash: BlockHash::GENESIS,
            view: ViewNumber(0),
            proposer: ValidatorId(0),
            timestamp: 0,
            payload: vec![],
            app_hash: BlockHash::GENESIS,
            evidence: vec![],
            hash: BlockHash::GENESIS,
        }
    }

    #[test]
    fn does_not_retry_on_commit_after_written_request() {
        let sock_path = test_socket_path("n");
        let _ = std::fs::remove_file(&sock_path);
        let listener = UnixListener::bind(&sock_path).expect("bind test socket");
        let accepted = Arc::new(AtomicUsize::new(0));
        let accepted_for_thread = Arc::clone(&accepted);

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept first client");
            let _ = read_frame_sync(&mut stream).expect("read first frame");
            accepted_for_thread.fetch_add(1, Ordering::SeqCst);
            drop(stream);

            listener
                .set_nonblocking(true)
                .expect("set nonblocking listener");
            let deadline = Instant::now() + Duration::from_millis(200);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = read_frame_sync(&mut stream);
                        accepted_for_thread.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        let client = IpcApplicationClient::new(&sock_path);
        let vs = make_validator_set();
        let ctx = make_block_context(&vs);
        let err = client.on_commit(&make_block(), &ctx).unwrap_err();
        assert!(err.to_string().contains("not retrying"));

        server.join().expect("server thread");
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_file(&sock_path);
    }
}
