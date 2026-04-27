use std::io;

use hotmint_abci_proto::pb;
use hotmint_consensus::application::AppInfo;
use hotmint_consensus::liveness::OfflineEvidence;
use hotmint_types::context::{OwnedBlockContext, TxContext};
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::sync::{ChunkApplyResult, SnapshotInfo, SnapshotOfferResult};
use hotmint_types::validator::ValidatorId;
use hotmint_types::validator_update::EndBlockResponse;
use hotmint_types::{Block, BlockHash, Height};
use prost::Message;

/// IPC request sent from the consensus engine (client) to the application (server).
#[derive(Debug)]
pub enum Request {
    Info,
    InitChain(Vec<u8>),
    CreatePayload(OwnedBlockContext),
    ValidateBlock {
        block: Block,
        ctx: OwnedBlockContext,
    },
    ValidateTx {
        tx: Vec<u8>,
        ctx: Option<TxContext>,
    },
    ExecuteBlock {
        txs: Vec<Vec<u8>>,
        ctx: OwnedBlockContext,
    },
    OnCommit {
        block: Block,
        ctx: OwnedBlockContext,
    },
    OnEvidence(EquivocationProof),
    OnOfflineValidators(Vec<OfflineEvidence>),
    ExtendVote {
        block: Block,
        ctx: OwnedBlockContext,
    },
    VerifyVoteExtension {
        extension: Vec<u8>,
        block_hash: BlockHash,
        validator: ValidatorId,
    },
    Query {
        path: String,
        data: Vec<u8>,
    },
    ListSnapshots,
    LoadSnapshotChunk {
        height: Height,
        chunk_index: u32,
    },
    OfferSnapshot(SnapshotInfo),
    ApplySnapshotChunk {
        chunk: Vec<u8>,
        chunk_index: u32,
    },
    TracksAppHash,
}

/// IPC response sent from the application (server) back to the consensus engine (client).
#[derive(Debug)]
pub enum Response {
    Info(AppInfo),
    InitChain(Result<BlockHash, String>),
    CreatePayload(Vec<u8>),
    ValidateBlock(bool),
    ValidateTx {
        ok: bool,
        priority: u64,
        gas_wanted: u64,
    },
    ExecuteBlock(Result<EndBlockResponse, String>),
    OnCommit(Result<(), String>),
    OnEvidence(Result<(), String>),
    OnOfflineValidators(Result<(), String>),
    ExtendVote(Option<Vec<u8>>),
    VerifyVoteExtension(bool),
    Query(Result<hotmint_types::QueryResponse, String>),
    ListSnapshots(Vec<SnapshotInfo>),
    LoadSnapshotChunk(Vec<u8>),
    OfferSnapshot(SnapshotOfferResult),
    ApplySnapshotChunk(ChunkApplyResult),
    TracksAppHash(bool),
}

// ---- Protobuf encode/decode for Request ----

pub fn encode_request(req: &Request) -> Vec<u8> {
    let proto_req = match req {
        Request::Info => pb::Request {
            request: Some(pb::request::Request::Info(pb::InfoRequest {})),
        },
        Request::InitChain(app_state) => pb::Request {
            request: Some(pb::request::Request::InitChain(pb::InitChainRequest {
                app_state: app_state.clone(),
            })),
        },
        Request::CreatePayload(ctx) => pb::Request {
            request: Some(pb::request::Request::CreatePayload(ctx.into())),
        },
        Request::ValidateBlock { block, ctx } => pb::Request {
            request: Some(pb::request::Request::ValidateBlock(
                pb::ValidateBlockRequest {
                    block: Some(block.into()),
                    ctx: Some(ctx.into()),
                },
            )),
        },
        Request::ValidateTx { tx, ctx } => pb::Request {
            request: Some(pb::request::Request::ValidateTx(pb::ValidateTxRequest {
                tx: tx.clone(),
                ctx: ctx.as_ref().map(|c| c.into()),
            })),
        },
        Request::ExecuteBlock { txs, ctx } => pb::Request {
            request: Some(pb::request::Request::ExecuteBlock(
                pb::ExecuteBlockRequest {
                    txs: txs.clone(),
                    ctx: Some(ctx.into()),
                },
            )),
        },
        Request::OnCommit { block, ctx } => pb::Request {
            request: Some(pb::request::Request::OnCommit(pb::OnCommitRequest {
                block: Some(block.into()),
                ctx: Some(ctx.into()),
            })),
        },
        Request::OnEvidence(proof) => pb::Request {
            request: Some(pb::request::Request::OnEvidence(proof.into())),
        },
        Request::OnOfflineValidators(offline) => pb::Request {
            request: Some(pb::request::Request::OnOfflineValidators(
                pb::OnOfflineValidatorsRequest {
                    offline: offline.iter().map(offline_evidence_to_proto).collect(),
                },
            )),
        },
        Request::ExtendVote { block, ctx } => pb::Request {
            request: Some(pb::request::Request::ExtendVote(pb::ExtendVoteRequest {
                block: Some(block.into()),
                ctx: Some(ctx.into()),
            })),
        },
        Request::VerifyVoteExtension {
            extension,
            block_hash,
            validator,
        } => pb::Request {
            request: Some(pb::request::Request::VerifyVoteExtension(
                pb::VerifyVoteExtensionRequest {
                    extension: extension.clone(),
                    block_hash: block_hash.0.to_vec(),
                    validator: validator.0,
                },
            )),
        },
        Request::Query { path, data } => pb::Request {
            request: Some(pb::request::Request::Query(pb::QueryRequest {
                path: path.clone(),
                data: data.clone(),
            })),
        },
        Request::ListSnapshots => pb::Request {
            request: Some(pb::request::Request::ListSnapshots(
                pb::ListSnapshotsRequest {},
            )),
        },
        Request::LoadSnapshotChunk {
            height,
            chunk_index,
        } => pb::Request {
            request: Some(pb::request::Request::LoadSnapshotChunk(
                pb::LoadSnapshotChunkRequest {
                    height: height.0,
                    chunk_index: *chunk_index,
                },
            )),
        },
        Request::OfferSnapshot(snapshot) => pb::Request {
            request: Some(pb::request::Request::OfferSnapshot(
                pb::OfferSnapshotRequest {
                    snapshot: Some(snapshot.into()),
                },
            )),
        },
        Request::ApplySnapshotChunk { chunk, chunk_index } => pb::Request {
            request: Some(pb::request::Request::ApplySnapshotChunk(
                pb::ApplySnapshotChunkRequest {
                    chunk: chunk.clone(),
                    chunk_index: *chunk_index,
                },
            )),
        },
        Request::TracksAppHash => pb::Request {
            request: Some(pb::request::Request::TracksAppHash(
                pb::TracksAppHashRequest {},
            )),
        },
    };
    proto_req.encode_to_vec()
}

pub fn decode_request(buf: &[u8]) -> Result<Request, prost::DecodeError> {
    let proto_req = pb::Request::decode(buf)?;
    let req = match proto_req
        .request
        .ok_or_else(|| prost::DecodeError::new("missing request oneof"))?
    {
        pb::request::Request::Info(_) => Request::Info,
        pb::request::Request::InitChain(r) => Request::InitChain(r.app_state),
        pb::request::Request::CreatePayload(ctx) => Request::CreatePayload(ctx.try_into()?),
        pb::request::Request::ValidateBlock(r) => Request::ValidateBlock {
            block: r
                .block
                .ok_or_else(|| prost::DecodeError::new("missing block"))?
                .try_into()?,
            ctx: r
                .ctx
                .ok_or_else(|| prost::DecodeError::new("missing ctx"))?
                .try_into()?,
        },
        pb::request::Request::ValidateTx(r) => Request::ValidateTx {
            tx: r.tx,
            ctx: r.ctx.map(Into::into),
        },
        pb::request::Request::ExecuteBlock(r) => Request::ExecuteBlock {
            txs: r.txs,
            ctx: r
                .ctx
                .ok_or_else(|| prost::DecodeError::new("missing ctx"))?
                .try_into()?,
        },
        pb::request::Request::OnCommit(r) => Request::OnCommit {
            block: r
                .block
                .ok_or_else(|| prost::DecodeError::new("missing block"))?
                .try_into()?,
            ctx: r
                .ctx
                .ok_or_else(|| prost::DecodeError::new("missing ctx"))?
                .try_into()?,
        },
        pb::request::Request::OnEvidence(proof) => Request::OnEvidence(proof.try_into()?),
        pb::request::Request::OnOfflineValidators(r) => Request::OnOfflineValidators(
            r.offline
                .into_iter()
                .map(offline_evidence_from_proto)
                .collect::<Result<_, _>>()?,
        ),
        pb::request::Request::ExtendVote(r) => Request::ExtendVote {
            block: r
                .block
                .ok_or_else(|| prost::DecodeError::new("missing block"))?
                .try_into()?,
            ctx: r
                .ctx
                .ok_or_else(|| prost::DecodeError::new("missing ctx"))?
                .try_into()?,
        },
        pb::request::Request::VerifyVoteExtension(r) => Request::VerifyVoteExtension {
            extension: r.extension,
            block_hash: bytes_to_block_hash("verify_vote_extension.block_hash", &r.block_hash)?,
            validator: ValidatorId(r.validator),
        },
        pb::request::Request::Query(r) => Request::Query {
            path: r.path,
            data: r.data,
        },
        pb::request::Request::ListSnapshots(_) => Request::ListSnapshots,
        pb::request::Request::LoadSnapshotChunk(r) => Request::LoadSnapshotChunk {
            height: Height(r.height),
            chunk_index: r.chunk_index,
        },
        pb::request::Request::OfferSnapshot(r) => Request::OfferSnapshot(
            r.snapshot
                .ok_or_else(|| prost::DecodeError::new("missing snapshot"))?
                .try_into()?,
        ),
        pb::request::Request::ApplySnapshotChunk(r) => Request::ApplySnapshotChunk {
            chunk: r.chunk,
            chunk_index: r.chunk_index,
        },
        pb::request::Request::TracksAppHash(_) => Request::TracksAppHash,
    };
    Ok(req)
}

// ---- Protobuf encode/decode for Response ----

pub fn encode_response(resp: &Response) -> Vec<u8> {
    let proto_resp = match resp {
        Response::Info(info) => pb::Response {
            response: Some(pb::response::Response::Info(pb::InfoResponse {
                info: Some(app_info_to_proto(info)),
            })),
        },
        Response::InitChain(result) => pb::Response {
            response: Some(pb::response::Response::InitChain(pb::InitChainResponse {
                app_hash: result
                    .as_ref()
                    .ok()
                    .map(|hash| hash.0.to_vec())
                    .unwrap_or_default(),
                error: result.as_ref().err().cloned().unwrap_or_default(),
            })),
        },
        Response::CreatePayload(payload) => pb::Response {
            response: Some(pb::response::Response::CreatePayload(
                pb::CreatePayloadResponse {
                    payload: payload.clone(),
                },
            )),
        },
        Response::ValidateBlock(ok) => pb::Response {
            response: Some(pb::response::Response::ValidateBlock(
                pb::ValidateBlockResponse { ok: *ok },
            )),
        },
        Response::ValidateTx {
            ok,
            priority,
            gas_wanted,
        } => pb::Response {
            response: Some(pb::response::Response::ValidateTx(pb::ValidateTxResponse {
                ok: *ok,
                priority: *priority,
                gas_wanted: *gas_wanted,
            })),
        },
        Response::ExecuteBlock(result) => pb::Response {
            response: Some(pb::response::Response::ExecuteBlock(
                pb::ExecuteBlockResponse {
                    result: result.as_ref().ok().map(|r| r.into()),
                    error: result.as_ref().err().cloned().unwrap_or_default(),
                },
            )),
        },
        Response::OnCommit(result) => pb::Response {
            response: Some(pb::response::Response::OnCommit(pb::OnCommitResponse {
                error: result.as_ref().err().cloned().unwrap_or_default(),
            })),
        },
        Response::OnEvidence(result) => pb::Response {
            response: Some(pb::response::Response::OnEvidence(pb::OnEvidenceResponse {
                error: result.as_ref().err().cloned().unwrap_or_default(),
            })),
        },
        Response::OnOfflineValidators(result) => pb::Response {
            response: Some(pb::response::Response::OnOfflineValidators(
                pb::OnOfflineValidatorsResponse {
                    error: result.as_ref().err().cloned().unwrap_or_default(),
                },
            )),
        },
        Response::ExtendVote(extension) => pb::Response {
            response: Some(pb::response::Response::ExtendVote(pb::ExtendVoteResponse {
                extension: extension.clone().unwrap_or_default(),
                has_extension: extension.is_some(),
            })),
        },
        Response::VerifyVoteExtension(ok) => pb::Response {
            response: Some(pb::response::Response::VerifyVoteExtension(
                pb::VerifyVoteExtensionResponse { ok: *ok },
            )),
        },
        Response::Query(result) => pb::Response {
            response: Some(pb::response::Response::Query(pb::QueryResponse {
                data: result
                    .as_ref()
                    .ok()
                    .map(|r| r.data.clone())
                    .unwrap_or_default(),
                error: result.as_ref().err().cloned().unwrap_or_default(),
                proof: result
                    .as_ref()
                    .ok()
                    .and_then(|r| r.proof.clone())
                    .unwrap_or_default(),
                height: result.as_ref().ok().map(|r| r.height).unwrap_or(0),
            })),
        },
        Response::ListSnapshots(snapshots) => pb::Response {
            response: Some(pb::response::Response::ListSnapshots(
                pb::ListSnapshotsResponse {
                    snapshots: snapshots.iter().map(pb::SnapshotInfo::from).collect(),
                },
            )),
        },
        Response::LoadSnapshotChunk(data) => pb::Response {
            response: Some(pb::response::Response::LoadSnapshotChunk(
                pb::LoadSnapshotChunkResponse { data: data.clone() },
            )),
        },
        Response::OfferSnapshot(result) => pb::Response {
            response: Some(pb::response::Response::OfferSnapshot(
                pb::OfferSnapshotResponse {
                    result: snapshot_offer_to_code(result),
                },
            )),
        },
        Response::ApplySnapshotChunk(result) => pb::Response {
            response: Some(pb::response::Response::ApplySnapshotChunk(
                pb::ApplySnapshotChunkResponse {
                    result: chunk_apply_to_code(result),
                },
            )),
        },
        Response::TracksAppHash(tracks) => pb::Response {
            response: Some(pb::response::Response::TracksAppHash(
                pb::TracksAppHashResponse { tracks: *tracks },
            )),
        },
    };
    proto_resp.encode_to_vec()
}

pub fn decode_response(buf: &[u8]) -> Result<Response, prost::DecodeError> {
    let proto_resp = pb::Response::decode(buf)?;
    let resp = match proto_resp
        .response
        .ok_or_else(|| prost::DecodeError::new("missing response oneof"))?
    {
        pb::response::Response::Info(r) => Response::Info(app_info_from_proto(
            r.info
                .ok_or_else(|| prost::DecodeError::new("missing app info"))?,
        )?),
        pb::response::Response::InitChain(r) => {
            if r.error.is_empty() {
                Response::InitChain(Ok(bytes_to_block_hash("init_chain.app_hash", &r.app_hash)?))
            } else {
                Response::InitChain(Err(r.error))
            }
        }
        pb::response::Response::CreatePayload(r) => Response::CreatePayload(r.payload),
        pb::response::Response::ValidateBlock(r) => Response::ValidateBlock(r.ok),
        pb::response::Response::ValidateTx(r) => Response::ValidateTx {
            ok: r.ok,
            priority: r.priority,
            gas_wanted: r.gas_wanted,
        },
        pb::response::Response::ExecuteBlock(r) => {
            if r.error.is_empty() {
                let ebr = r
                    .result
                    .ok_or_else(|| {
                        prost::DecodeError::new("missing execute_block result for success")
                    })?
                    .try_into()?;
                Response::ExecuteBlock(Ok(ebr))
            } else {
                Response::ExecuteBlock(Err(r.error))
            }
        }
        pb::response::Response::OnCommit(r) => {
            if r.error.is_empty() {
                Response::OnCommit(Ok(()))
            } else {
                Response::OnCommit(Err(r.error))
            }
        }
        pb::response::Response::OnEvidence(r) => {
            if r.error.is_empty() {
                Response::OnEvidence(Ok(()))
            } else {
                Response::OnEvidence(Err(r.error))
            }
        }
        pb::response::Response::OnOfflineValidators(r) => {
            if r.error.is_empty() {
                Response::OnOfflineValidators(Ok(()))
            } else {
                Response::OnOfflineValidators(Err(r.error))
            }
        }
        pb::response::Response::ExtendVote(r) => Response::ExtendVote(if r.has_extension {
            Some(r.extension)
        } else {
            None
        }),
        pb::response::Response::VerifyVoteExtension(r) => Response::VerifyVoteExtension(r.ok),
        pb::response::Response::Query(r) => {
            if r.error.is_empty() {
                Response::Query(Ok(hotmint_types::QueryResponse {
                    data: r.data,
                    proof: if r.proof.is_empty() {
                        None
                    } else {
                        Some(r.proof)
                    },
                    height: r.height,
                }))
            } else {
                Response::Query(Err(r.error))
            }
        }
        pb::response::Response::ListSnapshots(r) => Response::ListSnapshots(
            r.snapshots
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        ),
        pb::response::Response::LoadSnapshotChunk(r) => Response::LoadSnapshotChunk(r.data),
        pb::response::Response::OfferSnapshot(r) => {
            Response::OfferSnapshot(snapshot_offer_from_code(r.result)?)
        }
        pb::response::Response::ApplySnapshotChunk(r) => {
            Response::ApplySnapshotChunk(chunk_apply_from_code(r.result)?)
        }
        pb::response::Response::TracksAppHash(r) => Response::TracksAppHash(r.tracks),
    };
    Ok(resp)
}

fn app_info_to_proto(info: &AppInfo) -> pb::AppInfo {
    pb::AppInfo {
        last_block_height: info.last_block_height.0,
        last_block_app_hash: info.last_block_app_hash.0.to_vec(),
    }
}

fn app_info_from_proto(info: pb::AppInfo) -> Result<AppInfo, prost::DecodeError> {
    Ok(AppInfo {
        last_block_height: Height(info.last_block_height),
        last_block_app_hash: bytes_to_block_hash(
            "app_info.last_block_app_hash",
            &info.last_block_app_hash,
        )?,
    })
}

fn offline_evidence_to_proto(evidence: &OfflineEvidence) -> pb::OfflineEvidence {
    pb::OfflineEvidence {
        validator: evidence.validator.0,
        missed_commits: evidence.missed_commits,
        total_commits: evidence.total_commits,
        evidence_height: evidence.evidence_height.0,
    }
}

fn offline_evidence_from_proto(
    evidence: pb::OfflineEvidence,
) -> Result<OfflineEvidence, prost::DecodeError> {
    Ok(OfflineEvidence {
        validator: ValidatorId(evidence.validator),
        missed_commits: evidence.missed_commits,
        total_commits: evidence.total_commits,
        evidence_height: Height(evidence.evidence_height),
    })
}

fn snapshot_offer_to_code(result: &SnapshotOfferResult) -> u32 {
    match result {
        SnapshotOfferResult::Accept => 0,
        SnapshotOfferResult::Reject => 1,
        SnapshotOfferResult::Abort => 2,
    }
}

fn snapshot_offer_from_code(code: u32) -> Result<SnapshotOfferResult, prost::DecodeError> {
    match code {
        0 => Ok(SnapshotOfferResult::Accept),
        1 => Ok(SnapshotOfferResult::Reject),
        2 => Ok(SnapshotOfferResult::Abort),
        other => Err(prost::DecodeError::new(format!(
            "invalid snapshot offer result: {other}"
        ))),
    }
}

fn chunk_apply_to_code(result: &ChunkApplyResult) -> u32 {
    match result {
        ChunkApplyResult::Accept => 0,
        ChunkApplyResult::Retry => 1,
        ChunkApplyResult::Abort => 2,
    }
}

fn chunk_apply_from_code(code: u32) -> Result<ChunkApplyResult, prost::DecodeError> {
    match code {
        0 => Ok(ChunkApplyResult::Accept),
        1 => Ok(ChunkApplyResult::Retry),
        2 => Ok(ChunkApplyResult::Abort),
        other => Err(prost::DecodeError::new(format!(
            "invalid chunk apply result: {other}"
        ))),
    }
}

fn bytes_to_block_hash(field: &str, bytes: &[u8]) -> Result<BlockHash, prost::DecodeError> {
    let hash: [u8; 32] = bytes.try_into().map_err(|_| {
        prost::DecodeError::new(format!("{field}: expected 32 bytes, got {}", bytes.len()))
    })?;
    Ok(BlockHash(hash))
}

/// Maximum IPC frame size (64 MB).
const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

/// Write a length-prefixed frame to an async writer.
pub async fn write_frame(
    writer: &mut (impl tokio::io::AsyncWriteExt + Unpin),
    payload: &[u8],
) -> io::Result<()> {
    if payload.len() > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame size {} exceeds max {MAX_FRAME_SIZE}", payload.len()),
        ));
    }
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

/// Read a length-prefixed frame from an async reader.
pub async fn read_frame(
    reader: &mut (impl tokio::io::AsyncReadExt + Unpin),
) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame size {len} exceeds max {MAX_FRAME_SIZE}"),
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_block_success_requires_result() {
        let response = pb::Response {
            response: Some(pb::response::Response::ExecuteBlock(
                pb::ExecuteBlockResponse {
                    result: None,
                    error: String::new(),
                },
            )),
        };
        let err = decode_response(&response.encode_to_vec()).unwrap_err();
        assert!(err.to_string().contains("missing execute_block result"));
    }
}
