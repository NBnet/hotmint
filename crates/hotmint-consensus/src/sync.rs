//! Block sync: allows a node that is behind to catch up by requesting
//! missing blocks from peers and replaying the commit lifecycle.

use std::cmp;

use ruc::*;

use crate::application::Application;
use crate::commit;
use crate::engine::{StatePersistence, Wal};
use crate::store::BlockStore;
use hotmint_types::context::BlockContext;
use hotmint_types::epoch::Epoch;
use hotmint_types::sync::{MAX_SYNC_BATCH, SyncRequest, SyncResponse};
use hotmint_types::vote::{Vote, VoteType};
use hotmint_types::{
    AggregateSignature, Block, BlockHash, EpochNumber, Height, QuorumCertificate, ValidatorSet,
    Verifier, ViewNumber,
};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tracing::{info, warn};

const SYNC_TIMEOUT: Duration = Duration::from_secs(10);

struct SyncVoteAggregate<'a> {
    validator_set: &'a ValidatorSet,
    epoch: EpochNumber,
    view: ViewNumber,
    block_hash: &'a BlockHash,
    vote_type: VoteType,
    aggregate_signature: &'a AggregateSignature,
}

fn verify_vote_aggregate(
    chain_id_hash: &[u8; 32],
    verifier: &dyn Verifier,
    aggregate: SyncVoteAggregate<'_>,
) -> bool {
    let validator_set = aggregate.validator_set;
    let aggregate_signature = aggregate.aggregate_signature;
    if aggregate_signature.signers.len() != validator_set.validator_count() {
        return false;
    }

    let mut sig_idx = 0usize;
    for (idx, signed) in aggregate_signature.signers.iter().enumerate() {
        if !signed {
            continue;
        }
        let Some(validator) = validator_set.validators().get(idx) else {
            return false;
        };
        let Some(signature) = aggregate_signature.signatures.get(sig_idx) else {
            return false;
        };
        let bytes = Vote::signing_bytes(
            chain_id_hash,
            aggregate.epoch,
            aggregate.view,
            validator.id,
            aggregate.block_hash,
            aggregate.vote_type,
            None,
        );
        if !verifier.verify(&validator.public_key, &bytes, signature) {
            return false;
        }
        sig_idx += 1;
    }

    sig_idx == aggregate_signature.signatures.len()
        && hotmint_crypto::has_quorum(validator_set, aggregate_signature)
}

fn verify_vote_qc(
    chain_id_hash: &[u8; 32],
    verifier: &dyn hotmint_types::Verifier,
    current_epoch: &Epoch,
    qc: &QuorumCertificate,
) -> bool {
    qc.epoch == current_epoch.number
        && verify_vote_aggregate(
            chain_id_hash,
            verifier,
            SyncVoteAggregate {
                validator_set: &current_epoch.validator_set,
                epoch: qc.epoch,
                view: qc.view,
                block_hash: &qc.block_hash,
                vote_type: VoteType::Vote,
                aggregate_signature: &qc.aggregate_signature,
            },
        )
}

/// Mutable state needed by block sync and replay.
pub struct SyncState<'a> {
    pub store: &'a mut dyn BlockStore,
    pub app: &'a dyn Application,
    pub current_epoch: &'a mut Epoch,
    pub last_committed_height: &'a mut Height,
    pub last_app_hash: &'a mut BlockHash,
    pub chain_id_hash: &'a [u8; 32],
    /// A-1: Tracks pending epoch transitions across replay batches.
    pub pending_epoch: &'a mut Option<Epoch>,
    /// Previous validator epoch retained for in-flight certificates after replay.
    pub previous_epoch: &'a mut Option<Epoch>,
    /// Optional durable consensus-state checkpoint used during startup replay.
    pub persistence: Option<&'a mut dyn StatePersistence>,
    /// Optional WAL used to make sync replay crash-safe.
    pub wal: Option<&'a mut dyn Wal>,
}

/// Run block sync: request missing blocks from peers and replay them.
///
/// This should be called **before** the consensus engine starts.
/// Returns the updated (height, epoch) after syncing.
pub async fn sync_to_tip(
    state: &mut SyncState<'_>,
    request_tx: &mpsc::Sender<SyncRequest>,
    response_rx: &mut mpsc::Receiver<SyncResponse>,
) -> Result<()> {
    // First, get status from peer
    request_tx
        .send(SyncRequest::GetStatus)
        .await
        .map_err(|_| eg!("sync channel closed"))?;

    let peer_status = match timeout(SYNC_TIMEOUT, response_rx.recv()).await {
        Ok(Some(SyncResponse::Status {
            last_committed_height: peer_height,
            ..
        })) => peer_height,
        Ok(Some(SyncResponse::Error(e))) => return Err(eg!("peer error: {}", e)),
        Ok(Some(SyncResponse::Blocks(_))) => return Err(eg!("unexpected blocks response")),
        Ok(Some(SyncResponse::Snapshots(_))) => return Err(eg!("unexpected snapshots response")),
        Ok(Some(SyncResponse::SnapshotChunk { .. })) => {
            return Err(eg!("unexpected snapshot chunk response"));
        }
        Ok(None) => return Err(eg!("sync channel closed")),
        Err(_) => {
            info!("sync status request timed out, starting from current state");
            return Ok(());
        }
    };

    if peer_status <= *state.last_committed_height {
        info!(
            our_height = state.last_committed_height.as_u64(),
            peer_height = peer_status.as_u64(),
            "already caught up"
        );
        return Ok(());
    }

    info!(
        our_height = state.last_committed_height.as_u64(),
        peer_height = peer_status.as_u64(),
        "starting block sync"
    );

    // A-3: Try snapshot sync first for large gaps. If the peer has snapshots
    // and the application accepts one, we skip most of the block replay.
    let gap = peer_status
        .as_u64()
        .saturating_sub(state.last_committed_height.as_u64());
    if gap > MAX_SYNC_BATCH {
        match sync_via_snapshot(state, request_tx, response_rx).await {
            Ok(true) => {
                info!(
                    height = state.last_committed_height.as_u64(),
                    "snapshot sync succeeded, continuing with block sync for remaining blocks"
                );
                // Fall through to block sync for any blocks after the snapshot
            }
            Ok(false) => {
                info!("no suitable snapshot available, using full block sync");
            }
            Err(e) => {
                warn!(error = %e, "snapshot sync failed, falling back to block sync");
            }
        }
    }

    // If we're caught up after snapshot, return early
    if *state.last_committed_height >= peer_status {
        info!(
            height = state.last_committed_height.as_u64(),
            "caught up via snapshot"
        );
        return Ok(());
    }

    // Pipelined sync: prefetch the next batch while replaying the current one.
    // This overlaps network latency with CPU-bound block execution.

    // Request first batch
    let first_from = Height(state.last_committed_height.as_u64() + 1);
    let first_to = Height(cmp::min(
        first_from.as_u64() + MAX_SYNC_BATCH - 1,
        peer_status.as_u64(),
    ));
    request_tx
        .send(SyncRequest::GetBlocks {
            from_height: first_from,
            to_height: first_to,
        })
        .await
        .map_err(|_| eg!("sync channel closed"))?;

    loop {
        // Wait for current batch
        let blocks = match timeout(SYNC_TIMEOUT, response_rx.recv()).await {
            Ok(Some(SyncResponse::Blocks(blocks))) => blocks,
            Ok(Some(SyncResponse::Error(e))) => return Err(eg!("peer error: {}", e)),
            Ok(Some(SyncResponse::Status { .. })) => return Err(eg!("unexpected status response")),
            Ok(Some(SyncResponse::Snapshots(_))) => {
                return Err(eg!("unexpected snapshots response"));
            }
            Ok(Some(SyncResponse::SnapshotChunk { .. })) => {
                return Err(eg!("unexpected snapshot chunk response"));
            }
            Ok(None) => return Err(eg!("sync channel closed")),
            Err(_) => return Err(eg!("sync request timed out")),
        };

        if blocks.is_empty() {
            break;
        }

        // Prefetch next batch BEFORE replaying current one (pipeline overlap).
        let synced_through = state.last_committed_height.as_u64() + blocks.len() as u64;
        let needs_more = synced_through < peer_status.as_u64();
        if needs_more {
            let next_from = Height(
                blocks
                    .last()
                    .map(|(b, _)| b.height.as_u64() + 1)
                    .unwrap_or(first_from.as_u64()),
            );
            let next_to = Height(cmp::min(
                next_from.as_u64() + MAX_SYNC_BATCH - 1,
                peer_status.as_u64(),
            ));
            let _ = request_tx
                .send(SyncRequest::GetBlocks {
                    from_height: next_from,
                    to_height: next_to,
                })
                .await;
        }

        // Replay current batch while the next one is being fetched.
        // A-1: Propagate pending epoch from replay so it survives across
        // batches and is available to the caller after sync completes.
        if let Some(epoch) = replay_blocks(&blocks, state)? {
            *state.pending_epoch = Some(epoch);
        }

        info!(
            synced_to = state.last_committed_height.as_u64(),
            target = peer_status.as_u64(),
            "sync progress"
        );

        if *state.last_committed_height >= peer_status || !needs_more {
            break;
        }
    }

    info!(
        height = state.last_committed_height.as_u64(),
        epoch = %state.current_epoch.number,
        "block sync complete"
    );
    Ok(())
}

/// Query a peer's status via the sync channel. Returns `None` on timeout/error.
pub async fn query_peer_status(
    request_tx: &mpsc::Sender<SyncRequest>,
    response_rx: &mut mpsc::Receiver<SyncResponse>,
) -> Option<Height> {
    request_tx.send(SyncRequest::GetStatus).await.ok()?;
    match timeout(SYNC_TIMEOUT, response_rx.recv()).await {
        Ok(Some(SyncResponse::Status {
            last_committed_height,
            ..
        })) => Some(last_committed_height),
        _ => None,
    }
}

/// Attempt state sync via snapshots. Returns `Ok(true)` if successful,
/// `Ok(false)` if no snapshots are available (caller should fall back to block sync).
pub async fn sync_via_snapshot(
    state: &mut SyncState<'_>,
    request_tx: &mpsc::Sender<SyncRequest>,
    response_rx: &mut mpsc::Receiver<SyncResponse>,
) -> Result<bool> {
    // 1. Request snapshot list from peer
    request_tx
        .send(SyncRequest::GetSnapshots)
        .await
        .map_err(|_| eg!("sync channel closed"))?;

    // 2. Wait for the snapshot list (10s timeout)
    let snapshots = match timeout(SYNC_TIMEOUT, response_rx.recv()).await {
        Ok(Some(SyncResponse::Snapshots(list))) => list,
        Ok(Some(SyncResponse::Error(e))) => return Err(eg!("peer error: {}", e)),
        Ok(Some(_)) => return Err(eg!("unexpected response to GetSnapshots")),
        Ok(None) => return Err(eg!("sync channel closed")),
        Err(_) => {
            info!("snapshot list request timed out");
            return Ok(false);
        }
    };

    // 3. If no snapshots available, fall back to block sync
    if snapshots.is_empty() {
        info!("peer has no snapshots, falling back to block sync");
        return Ok(false);
    }

    // 4. Pick the latest snapshot (highest height)
    let snapshot = snapshots
        .iter()
        .max_by_key(|s| s.height.as_u64())
        .unwrap()
        .clone();

    // Skip if we're already at or past this height
    if snapshot.height <= *state.last_committed_height {
        info!(
            snapshot_height = snapshot.height.as_u64(),
            our_height = state.last_committed_height.as_u64(),
            "snapshot not ahead of our state, falling back to block sync"
        );
        return Ok(false);
    }

    warn!(
        snapshot_height = snapshot.height.as_u64(),
        chunks = snapshot.chunks,
        "snapshot sync disabled: current Application API cannot stage/verify snapshots before live state mutation; falling back to block sync"
    );
    Ok(false)
}

/// Replay a batch of blocks: store them and run the application lifecycle.
/// Validates chain continuity (parent_hash linkage).
/// Replay synced blocks and return any pending epoch that hasn't reached its
/// `start_view` yet. The caller should store this as `pending_epoch` in the
/// engine so the epoch transition fires at the correct view.
pub fn replay_blocks(
    blocks: &[(Block, Option<hotmint_types::QuorumCertificate>)],
    state: &mut SyncState<'_>,
) -> Result<Option<Epoch>> {
    // H-7: Track pending epoch separately, matching the runtime's deferred
    // activation semantics. The new epoch only takes effect when we reach
    // its start_view, preventing validator set mismatches during replay.
    // A4-1: Load any pending epoch from a prior batch so cross-batch
    // epoch transitions are not lost.
    let mut pending_epoch: Option<Epoch> = state.pending_epoch.clone();

    for (block, qc) in blocks {
        // Stage epoch activation until the incoming block has been authenticated.
        // Invalid sync responses must not change the live epoch or consume a
        // transition needed by a retry against another peer.
        let activates_epoch = pending_epoch
            .as_ref()
            .is_some_and(|epoch| block.view >= epoch.start_view);
        let replay_epoch = if activates_epoch {
            pending_epoch.as_ref().unwrap().clone()
        } else {
            state.current_epoch.clone()
        };

        // Validate height continuity against the local committed cursor.
        let expected_height = state
            .last_committed_height
            .as_u64()
            .checked_add(1)
            .ok_or_else(|| {
                eg!(
                    "cannot replay block {}: last committed height overflow",
                    block.height.as_u64()
                )
            })?;
        if block.height.as_u64() != expected_height {
            return Err(eg!(
                "sync block height discontinuity: expected {}, got {}",
                expected_height,
                block.height.as_u64()
            ));
        }

        // Validate parent continuity against the committed block we are extending.
        let expected_parent = if *state.last_committed_height == Height::GENESIS {
            BlockHash::GENESIS
        } else {
            state
                .store
                .get_block_by_height(*state.last_committed_height)
                .ok_or_else(|| {
                    eg!(
                        "cannot replay height {}: missing last committed block at height {}",
                        block.height.as_u64(),
                        state.last_committed_height.as_u64()
                    )
                })?
                .hash
        };
        if block.parent_hash != expected_parent {
            return Err(eg!(
                "chain discontinuity at height {}: expected parent {}, got {}",
                block.height.as_u64(),
                expected_parent,
                block.parent_hash
            ));
        }

        // Verify commit QC if present (non-genesis blocks should have one)
        if let Some(cert) = qc {
            if cert.block_hash != block.hash {
                return Err(eg!(
                    "sync QC block_hash mismatch at height {}: QC={} block={}",
                    block.height.as_u64(),
                    cert.block_hash,
                    block.hash
                ));
            }
            // Verify QC aggregate signature and quorum against the epoch that formed it.
            let verifier = hotmint_crypto::Ed25519Verifier;
            if !verify_vote_qc(state.chain_id_hash, &verifier, &replay_epoch, cert) {
                return Err(eg!(
                    "sync QC verification failed at height {}",
                    block.height.as_u64()
                ));
            }
        } else if block.height.as_u64() > 0 {
            // Non-genesis blocks MUST have a commit QC — without one, the block
            // has not been certified by a 2/3 quorum and must be rejected.
            return Err(eg!(
                "sync block at height {} missing commit QC — refusing unverified block",
                block.height.as_u64()
            ));
        }

        // Defense-in-depth: verify the proposer is the correct leader for this view.
        // The QC already proves 2f+1 honest validators accepted this block (and they
        // checked the proposer), but we re-check here to catch corrupted sync data.
        if let Some(expected_leader) = replay_epoch.validator_set.leader_for_view(block.view)
            && block.proposer != expected_leader.id
        {
            return Err(eg!(
                "sync block at height {} has wrong proposer: expected {} for view {}, got {}",
                block.height.as_u64(),
                expected_leader.id,
                block.view,
                block.proposer
            ));
        }

        // Verify block hash integrity (includes app_hash in computation)
        let expected_hash = hotmint_crypto::compute_block_hash(block);
        if block.hash != expected_hash {
            return Err(eg!(
                "sync block hash mismatch at height {}: declared {} != computed {}",
                block.height.as_u64(),
                block.hash,
                expected_hash
            ));
        }

        // Verify app_hash matches local application state.
        // Skip when the application does not track state roots (e.g. NoopApplication),
        // so that fullnodes without an ABCI backend can sync from peers running real
        // applications that produce non-zero app_hash values.
        if state.app.tracks_app_hash() && block.app_hash != *state.last_app_hash {
            return Err(eg!(
                "sync block app_hash mismatch at height {}: block {} != local {}",
                block.height.as_u64(),
                block.app_hash,
                state.last_app_hash
            ));
        }

        commit::validate_block_evidence(
            block,
            &replay_epoch,
            state.chain_id_hash,
            &hotmint_crypto::Ed25519Verifier,
        )?;

        // Store the block and its commit QC (H-12: so the node can serve
        // commit proofs to other syncing peers and light clients).
        state.store.put_block(block.clone());
        if let Some(commit_qc) = qc {
            state.store.put_commit_qc(block.height, commit_qc.clone());
        }
        state.store.flush();

        // WAL discipline: the block and QC are durable before intent, and the
        // intent is fsynced before the application can be mutated.
        if let Some(wal) = state.wal.as_deref_mut() {
            wal.log_commit_intent(block.height).c(d!(
                "WAL: failed to fsync sync replay intent for height {}",
                block.height.as_u64()
            ))?;
        }

        // Run application lifecycle
        let ctx = BlockContext {
            height: block.height,
            view: block.view,
            proposer: block.proposer,
            epoch: replay_epoch.number,
            epoch_start_view: replay_epoch.start_view,
            validator_set: &replay_epoch.validator_set,
            timestamp: block.timestamp,
            vote_extensions: vec![],
        };

        if !state.app.validate_block(block, &ctx) {
            return Err(eg!(
                "app rejected synced block at height {}",
                block.height.as_u64()
            ));
        }

        if activates_epoch {
            *state.previous_epoch = Some(state.current_epoch.clone());
            pending_epoch = None;
        }
        *state.current_epoch = replay_epoch.clone();

        let response = commit::execute_committed_block(block, state.app, &ctx);

        *state.last_app_hash = if state.app.tracks_app_hash() {
            response.app_hash
        } else {
            // App does not compute state roots: carry the chain's authoritative
            // value forward so that the continuity check stays coherent.
            block.app_hash
        };

        // Handle epoch transitions — defer to start_view (H-7). Chain onto the
        // in-flight pending epoch (if any) so multiple updates committed within
        // this replay batch accumulate identically to the live commit path;
        // otherwise a synced node would derive a different validator set.
        if !response.validator_updates.is_empty() {
            pending_epoch = Some(
                commit::chained_pending_epoch(
                    state.current_epoch,
                    pending_epoch.as_ref(),
                    block.view,
                    &response.validator_updates,
                )
                .c(d!(
                    "invalid validator updates while replaying height {}",
                    block.height.as_u64()
                ))?,
            );
        }

        // Replay must build the same RPC indexes/results as live commits.
        for (index, tx) in commit::decode_payload(&block.payload).iter().enumerate() {
            state
                .store
                .put_tx_index(*blake3::hash(tx).as_bytes(), block.height, index as u32);
        }
        state.store.put_block_results(block.height, response);
        state.store.flush();

        *state.last_committed_height = block.height;
        *state.pending_epoch = pending_epoch.clone();
        persist_sync_checkpoint(state, pending_epoch.as_ref());

        if let Some(wal) = state.wal.as_deref_mut() {
            wal.log_commit_done(block.height).c(d!(
                "WAL: failed to fsync sync replay completion for height {}",
                block.height.as_u64()
            ))?;
        }
    }

    // Return any still-pending epoch for the caller to handle.
    *state.pending_epoch = pending_epoch.clone();
    Ok(pending_epoch)
}

fn persist_sync_checkpoint(state: &mut SyncState<'_>, pending_epoch: Option<&Epoch>) {
    if let Some(persistence) = state.persistence.as_deref_mut() {
        persistence.save_last_committed_height(*state.last_committed_height);
        persistence.save_current_epoch(state.current_epoch);
        persistence.save_previous_epoch(state.previous_epoch.as_ref());
        persistence.save_last_app_hash(*state.last_app_hash);
        persistence.save_pending_epoch(pending_epoch);
        persistence.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::NoopApplication;
    use crate::store::MemoryBlockStore;
    use hotmint_types::epoch::EpochNumber;
    use hotmint_types::{BlockHash, QuorumCertificate, ValidatorId, ViewNumber};

    const TEST_CHAIN: [u8; 32] = [0u8; 32];

    fn make_qc(block: &Block, signer: &hotmint_crypto::Ed25519Signer) -> QuorumCertificate {
        make_epoch_qc(block, signer, EpochNumber(0))
    }

    fn make_epoch_qc(
        block: &Block,
        signer: &hotmint_crypto::Ed25519Signer,
        epoch: EpochNumber,
    ) -> QuorumCertificate {
        let vote_bytes = hotmint_types::vote::Vote::signing_bytes(
            &TEST_CHAIN,
            epoch,
            block.view,
            hotmint_types::Signer::validator_id(signer),
            &block.hash,
            hotmint_types::vote::VoteType::Vote,
            None,
        );
        let sig = hotmint_types::Signer::sign(signer, &vote_bytes);
        let mut agg = hotmint_types::AggregateSignature::new(1);
        agg.add(0, sig).unwrap();
        QuorumCertificate {
            block_hash: block.hash,
            view: block.view,
            aggregate_signature: agg,
            epoch,
        }
    }

    fn make_block(height: u64, parent: BlockHash) -> Block {
        let mut block = Block {
            height: Height(height),
            parent_hash: parent,
            view: ViewNumber(height),
            proposer: ValidatorId(0),
            timestamp: 0,
            payload: vec![],
            app_hash: BlockHash::GENESIS,
            evidence: Vec::new(),
            hash: BlockHash::GENESIS, // placeholder
        };
        block.hash = hotmint_crypto::compute_block_hash(&block);
        block
    }

    #[test]
    fn committed_evidence_has_identical_live_and_replay_effects() {
        use hotmint_types::{DoubleCertificate, EndBlockResponse, EquivocationProof, Signer};
        use std::sync::atomic::{AtomicU8, Ordering};
        struct EvidenceApp(AtomicU8);
        impl Application for EvidenceApp {
            fn on_evidence(&self, _: &EquivocationProof) -> Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            fn execute_block(&self, _: &[&[u8]], _: &BlockContext) -> Result<EndBlockResponse> {
                Ok(EndBlockResponse {
                    app_hash: BlockHash([self.0.load(Ordering::SeqCst); 32]),
                    ..Default::default()
                })
            }
        }
        #[derive(Default)]
        struct ReplayStore {
            inner: MemoryBlockStore,
            indexed: Vec<([u8; 32], Height, u32)>,
            results: Vec<(Height, EndBlockResponse)>,
        }
        impl BlockStore for ReplayStore {
            fn put_block(&mut self, block: Block) {
                self.inner.put_block(block);
            }
            fn get_block(&self, hash: &BlockHash) -> Option<Block> {
                self.inner.get_block(hash)
            }
            fn get_block_by_height(&self, height: Height) -> Option<Block> {
                self.inner.get_block_by_height(height)
            }
            fn put_tx_index(&mut self, hash: [u8; 32], height: Height, index: u32) {
                self.indexed.push((hash, height, index));
            }
            fn put_block_results(&mut self, height: Height, response: EndBlockResponse) {
                self.results.push((height, response));
            }
        }
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let mut epoch = Epoch::genesis(ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: signer.public_key(),
            power: 1,
        }]));
        let hash_a = BlockHash([10; 32]);
        let hash_b = BlockHash([20; 32]);
        let sign = |hash: &BlockHash| {
            signer.sign(&Vote::signing_bytes(
                &TEST_CHAIN,
                EpochNumber(0),
                ViewNumber(1),
                ValidatorId(0),
                hash,
                VoteType::Vote,
                None,
            ))
        };
        let proof = EquivocationProof {
            validator: ValidatorId(0),
            epoch: EpochNumber(0),
            view: ViewNumber(1),
            vote_type: VoteType::Vote,
            block_hash_a: hash_a,
            signature_a: sign(&hash_a),
            block_hash_b: hash_b,
            signature_b: sign(&hash_b),
            extension_a: None,
            extension_b: None,
        };
        let mut block = make_block(1, BlockHash::GENESIS);
        block.evidence.push(proof);
        block.payload = [2u32.to_le_bytes().as_slice(), b"tx"].concat();
        block.hash = block.compute_hash();
        let qc = make_qc(&block, &signer);
        let mut live_store = MemoryBlockStore::new();
        live_store.put_block(block.clone());
        let live_app = EvidenceApp(AtomicU8::new(0));
        let mut live_height = Height::GENESIS;
        let result = commit::try_commit(
            &DoubleCertificate {
                inner_qc: qc.clone(),
                outer_qc: qc.clone(),
                vote_extensions: vec![],
            },
            &live_store,
            &live_app,
            &mut live_height,
            &epoch,
        )
        .unwrap();
        let replay_app = EvidenceApp(AtomicU8::new(0));
        let mut store = ReplayStore::default();
        let mut height = Height::GENESIS;
        let mut hash = BlockHash::GENESIS;
        let mut pending = None;
        replay_blocks(
            &[(block.clone(), Some(qc))],
            &mut SyncState {
                store: &mut store,
                app: &replay_app,
                current_epoch: &mut epoch,
                last_committed_height: &mut height,
                last_app_hash: &mut hash,
                chain_id_hash: &TEST_CHAIN,
                pending_epoch: &mut pending,
                previous_epoch: &mut None,
                persistence: None,
                wal: None,
            },
        )
        .unwrap();
        assert_eq!(hash, BlockHash([1; 32]));
        assert_eq!(hash, result.last_app_hash);
        assert_eq!(
            store.indexed,
            vec![(*blake3::hash(b"tx").as_bytes(), Height(1), 0)]
        );
        assert_eq!(store.results.len(), 1);
        assert_eq!(store.results[0].1.app_hash, hash);
        assert_eq!(live_app.0.load(Ordering::SeqCst), 1);
        assert_eq!(replay_app.0.load(Ordering::SeqCst), 1);
        block.evidence[0].signature_a = signer.sign(b"invalid evidence");
        assert!(
            commit::validate_block_evidence(
                &block,
                &epoch,
                &TEST_CHAIN,
                &hotmint_crypto::Ed25519Verifier
            )
            .is_err()
        );
    }

    #[test]
    fn rejected_sync_block_does_not_activate_pending_epoch() {
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let mut epoch = Epoch::genesis(ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: hotmint_types::Signer::public_key(&signer),
            power: 1,
        }]));
        let mut pending = Some(Epoch::new(
            EpochNumber(1),
            ViewNumber(2),
            epoch.validator_set.clone(),
        ));
        let mut store = MemoryBlockStore::new();
        let mut height = Height::GENESIS;
        let mut hash = BlockHash::GENESIS;
        let block = make_block(2, BlockHash::GENESIS);
        assert!(
            replay_blocks(
                &[(block, None)],
                &mut SyncState {
                    store: &mut store,
                    app: &NoopApplication,
                    current_epoch: &mut epoch,
                    last_committed_height: &mut height,
                    last_app_hash: &mut hash,
                    chain_id_hash: &TEST_CHAIN,
                    pending_epoch: &mut pending,
                    previous_epoch: &mut None,
                    persistence: None,
                    wal: None,
                }
            )
            .is_err()
        );
        assert_eq!(epoch.number, EpochNumber(0));
        assert_eq!(pending.unwrap().number, EpochNumber(1));
        assert_eq!(height, Height::GENESIS);
    }

    #[test]
    fn test_replay_blocks_valid_chain() {
        let mut store = MemoryBlockStore::new();
        let app = NoopApplication;
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let vs = hotmint_types::ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: hotmint_types::Signer::public_key(&signer),
            power: 1,
        }]);
        let mut epoch = Epoch::genesis(vs);
        let mut height = Height::GENESIS;

        let b1 = make_block(1, BlockHash::GENESIS);
        let b2 = make_block(2, b1.hash);
        let b3 = make_block(3, b2.hash);

        let qc1 = make_qc(&b1, &signer);
        let qc2 = make_qc(&b2, &signer);
        let qc3 = make_qc(&b3, &signer);

        let blocks: Vec<_> = vec![(b1, Some(qc1)), (b2, Some(qc2)), (b3, Some(qc3))];
        let mut app_hash = BlockHash::GENESIS;
        let mut pending_epoch = None;
        let mut state = SyncState {
            store: &mut store,
            app: &app,
            current_epoch: &mut epoch,
            last_committed_height: &mut height,
            last_app_hash: &mut app_hash,
            chain_id_hash: &TEST_CHAIN,
            pending_epoch: &mut pending_epoch,
            previous_epoch: &mut None,
            persistence: None,
            wal: None,
        };
        replay_blocks(&blocks, &mut state).unwrap();
        assert_eq!(height, Height(3));
        assert!(store.get_block_by_height(Height(1)).is_some());
        assert!(store.get_block_by_height(Height(3)).is_some());
    }

    #[test]
    fn test_replay_blocks_rejects_missing_qc() {
        let mut store = MemoryBlockStore::new();
        let app = NoopApplication;
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let vs = hotmint_types::ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: hotmint_types::Signer::public_key(&signer),
            power: 1,
        }]);
        let mut epoch = Epoch::genesis(vs);
        let mut height = Height::GENESIS;

        let b1 = make_block(1, BlockHash::GENESIS);
        let qc1 = make_qc(&b1, &signer);
        let b2 = make_block(2, b1.hash);
        // Non-genesis block without QC should be rejected
        let blocks: Vec<_> = vec![(b1, Some(qc1)), (b2, None)];
        let mut app_hash = BlockHash::GENESIS;
        let mut pending_epoch = None;
        let mut state = SyncState {
            store: &mut store,
            app: &app,
            current_epoch: &mut epoch,
            last_committed_height: &mut height,
            last_app_hash: &mut app_hash,
            chain_id_hash: &TEST_CHAIN,
            pending_epoch: &mut pending_epoch,
            previous_epoch: &mut None,
            persistence: None,
            wal: None,
        };
        assert!(replay_blocks(&blocks, &mut state).is_err());
    }

    #[test]
    fn test_replay_blocks_broken_chain() {
        let mut store = MemoryBlockStore::new();
        let app = NoopApplication;
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let vs = hotmint_types::ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: hotmint_types::Signer::public_key(&signer),
            power: 1,
        }]);
        let mut epoch = Epoch::genesis(vs);
        let mut height = Height::GENESIS;

        let b1 = make_block(1, BlockHash::GENESIS);
        let b3 = make_block(3, BlockHash([99u8; 32])); // wrong parent

        let qc1 = make_qc(&b1, &signer);
        let qc3 = make_qc(&b3, &signer);
        let blocks: Vec<_> = vec![(b1, Some(qc1)), (b3, Some(qc3))];
        let mut app_hash = BlockHash::GENESIS;
        let mut pending_epoch = None;
        let mut state = SyncState {
            store: &mut store,
            app: &app,
            current_epoch: &mut epoch,
            last_committed_height: &mut height,
            last_app_hash: &mut app_hash,
            chain_id_hash: &TEST_CHAIN,
            pending_epoch: &mut pending_epoch,
            previous_epoch: &mut None,
            persistence: None,
            wal: None,
        };
        assert!(replay_blocks(&blocks, &mut state).is_err());
    }

    #[test]
    fn test_replay_blocks_rejects_height_gap_with_matching_parent() {
        let mut store = MemoryBlockStore::new();
        let app = NoopApplication;
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let vs = hotmint_types::ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: hotmint_types::Signer::public_key(&signer),
            power: 1,
        }]);
        let mut epoch = Epoch::genesis(vs);
        let mut height = Height::GENESIS;

        let b1 = make_block(1, BlockHash::GENESIS);
        let b3 = make_block(3, b1.hash);
        let qc1 = make_qc(&b1, &signer);
        let qc3 = make_qc(&b3, &signer);
        let b3_hash = b3.hash;
        let blocks: Vec<_> = vec![(b1, Some(qc1)), (b3, Some(qc3))];
        let mut app_hash = BlockHash::GENESIS;
        let mut pending_epoch = None;
        let mut state = SyncState {
            store: &mut store,
            app: &app,
            current_epoch: &mut epoch,
            last_committed_height: &mut height,
            last_app_hash: &mut app_hash,
            chain_id_hash: &TEST_CHAIN,
            pending_epoch: &mut pending_epoch,
            previous_epoch: &mut None,
            persistence: None,
            wal: None,
        };

        let err = replay_blocks(&blocks, &mut state).unwrap_err();
        assert!(format!("{err}").contains("height discontinuity"));
        assert_eq!(height, Height(1));
        assert!(store.get_block(&b3_hash).is_none());
    }

    #[derive(Default)]
    struct RecordingPersistence {
        heights: Vec<Height>,
        app_hashes: Vec<BlockHash>,
        epochs: Vec<EpochNumber>,
        pending_epochs: Vec<Option<EpochNumber>>,
        previous_epochs: Vec<Option<EpochNumber>>,
    }

    impl StatePersistence for RecordingPersistence {
        fn save_current_view(&mut self, _view: ViewNumber) {}
        fn save_locked_qc(&mut self, _qc: &QuorumCertificate) {}
        fn save_highest_qc(&mut self, _qc: &QuorumCertificate) {}
        fn save_last_committed_height(&mut self, height: Height) {
            self.heights.push(height);
        }
        fn save_current_epoch(&mut self, epoch: &Epoch) {
            self.epochs.push(epoch.number);
        }
        fn save_previous_epoch(&mut self, epoch: Option<&Epoch>) {
            self.previous_epochs.push(epoch.map(|e| e.number));
        }
        fn save_last_app_hash(&mut self, hash: BlockHash) {
            self.app_hashes.push(hash);
        }
        fn save_pending_epoch(&mut self, epoch: Option<&Epoch>) {
            self.pending_epochs.push(epoch.map(|e| e.number));
        }
        fn flush(&self) {}
    }

    struct RecordingWal {
        events: Vec<(&'static str, Height)>,
    }

    impl RecordingWal {
        fn new() -> Self {
            Self { events: Vec::new() }
        }
    }

    impl Wal for RecordingWal {
        fn log_commit_intent(&mut self, target_height: Height) -> std::io::Result<()> {
            self.events.push(("intent", target_height));
            Ok(())
        }

        fn log_commit_done(&mut self, target_height: Height) -> std::io::Result<()> {
            self.events.push(("done", target_height));
            Ok(())
        }
    }

    #[test]
    fn test_replay_blocks_records_wal_and_checkpoint() {
        let mut store = MemoryBlockStore::new();
        let app = NoopApplication;
        let signer = hotmint_crypto::Ed25519Signer::generate(ValidatorId(0));
        let vs = hotmint_types::ValidatorSet::new(vec![hotmint_types::ValidatorInfo {
            id: ValidatorId(0),
            public_key: hotmint_types::Signer::public_key(&signer),
            power: 1,
        }]);
        let mut epoch = Epoch::genesis(vs);
        let mut height = Height::GENESIS;
        let b1 = make_block(1, BlockHash::GENESIS);
        let qc1 = make_epoch_qc(&b1, &signer, EpochNumber(1));
        let blocks: Vec<_> = vec![(b1, Some(qc1))];
        let mut app_hash = BlockHash::GENESIS;
        let mut pending_epoch = Some(Epoch::new(
            EpochNumber(1),
            ViewNumber(1),
            epoch.validator_set.clone(),
        ));
        let mut previous_epoch = None;
        let mut persistence = RecordingPersistence::default();
        let mut wal = RecordingWal::new();

        {
            let mut state = SyncState {
                store: &mut store,
                app: &app,
                current_epoch: &mut epoch,
                last_committed_height: &mut height,
                last_app_hash: &mut app_hash,
                chain_id_hash: &TEST_CHAIN,
                pending_epoch: &mut pending_epoch,
                previous_epoch: &mut previous_epoch,
                persistence: Some(&mut persistence),
                wal: Some(&mut wal),
            };
            replay_blocks(&blocks, &mut state).unwrap();
        }

        assert_eq!(height, Height(1));
        assert_eq!(wal.events, vec![("intent", Height(1)), ("done", Height(1))]);
        assert_eq!(persistence.heights, vec![Height(1)]);
        assert_eq!(persistence.app_hashes, vec![BlockHash::GENESIS]);
        assert_eq!(persistence.epochs, vec![EpochNumber(1)]);
        assert_eq!(persistence.previous_epochs, vec![Some(EpochNumber(0))]);
        assert_eq!(previous_epoch.unwrap().number, EpochNumber(0));
        assert_eq!(persistence.pending_epochs, vec![None]);
    }
}
