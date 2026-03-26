//! Block sync: allows a node that is behind to catch up by requesting
//! missing blocks from peers and replaying the commit lifecycle.

use std::cmp;

use ruc::*;

use crate::application::Application;
use crate::commit;
use crate::store::BlockStore;
use hotmint_types::context::BlockContext;
use hotmint_types::epoch::Epoch;
use hotmint_types::sync::{
    ChunkApplyResult, MAX_SYNC_BATCH, SnapshotOfferResult, SyncRequest, SyncResponse,
};
use hotmint_types::{Block, BlockHash, Height, ViewNumber};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tracing::{info, warn};

const SYNC_TIMEOUT: Duration = Duration::from_secs(10);

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
    let gap = peer_status.as_u64().saturating_sub(state.last_committed_height.as_u64());
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

    info!(
        snapshot_height = snapshot.height.as_u64(),
        chunks = snapshot.chunks,
        "offering snapshot to application"
    );

    // 5. Offer the snapshot to the application
    let offer_result = state.app.offer_snapshot(&snapshot);
    match offer_result {
        SnapshotOfferResult::Accept => {}
        SnapshotOfferResult::Reject => {
            info!("application rejected snapshot, falling back to block sync");
            return Ok(false);
        }
        SnapshotOfferResult::Abort => {
            return Err(eg!("application aborted snapshot sync"));
        }
    }

    // 6. Download and apply chunks one by one
    for chunk_index in 0..snapshot.chunks {
        // Request chunk from peer
        request_tx
            .send(SyncRequest::GetSnapshotChunk {
                height: snapshot.height,
                chunk_index,
            })
            .await
            .map_err(|_| eg!("sync channel closed"))?;

        // Wait for chunk response
        let chunk_data = match timeout(SYNC_TIMEOUT, response_rx.recv()).await {
            Ok(Some(SyncResponse::SnapshotChunk { data, .. })) => data,
            Ok(Some(SyncResponse::Error(e))) => return Err(eg!("peer error: {}", e)),
            Ok(Some(_)) => return Err(eg!("unexpected response to GetSnapshotChunk")),
            Ok(None) => return Err(eg!("sync channel closed")),
            Err(_) => return Err(eg!("snapshot chunk request timed out")),
        };

        // Apply chunk to the application
        let apply_result = state.app.apply_snapshot_chunk(chunk_data, chunk_index);
        match apply_result {
            ChunkApplyResult::Accept => {
                info!(
                    chunk = chunk_index,
                    total = snapshot.chunks,
                    "applied snapshot chunk"
                );
            }
            ChunkApplyResult::Retry => {
                // For now, treat retry as a fatal error; a more sophisticated
                // implementation could retry the chunk download.
                warn!(
                    chunk = chunk_index,
                    "application requested chunk retry — aborting snapshot sync"
                );
                return Err(eg!(
                    "snapshot chunk {} apply requested retry (not yet supported)",
                    chunk_index
                ));
            }
            ChunkApplyResult::Abort => {
                return Err(eg!(
                    "application aborted snapshot sync at chunk {}",
                    chunk_index
                ));
            }
        }
    }

    // 7. Verify snapshot trust anchor (A-3): fetch the block+QC at the
    // snapshot height and verify the QC reaches quorum. The snapshot hash
    // must match the committed block's app_hash.
    request_tx
        .send(SyncRequest::GetBlocks {
            from_height: snapshot.height,
            to_height: snapshot.height,
        })
        .await
        .map_err(|_| eg!("sync channel closed"))?;

    let anchor_block = match timeout(SYNC_TIMEOUT, response_rx.recv()).await {
        Ok(Some(SyncResponse::Blocks(blocks))) if !blocks.is_empty() => blocks,
        Ok(Some(SyncResponse::Error(e))) => return Err(eg!("peer error fetching anchor: {}", e)),
        Ok(Some(_)) => return Err(eg!("unexpected response fetching snapshot anchor block")),
        Ok(None) => return Err(eg!("sync channel closed")),
        Err(_) => return Err(eg!("snapshot anchor block request timed out")),
    };

    let (block, qc_opt) = &anchor_block[0];
    let qc = qc_opt
        .as_ref()
        .ok_or_else(|| eg!("snapshot anchor block has no QC"))?;

    // Verify the QC signs this block
    if qc.block_hash != block.hash {
        return Err(eg!(
            "snapshot anchor QC block_hash {} != block hash {}",
            qc.block_hash,
            block.hash
        ));
    }

    // Bind snapshot hash to the verified block: the app_hash in the NEXT
    // committed block records the execution result of this block.  For the
    // snapshot height itself, the snapshot.hash must equal the block's
    // app_hash (which the application set during execute_block).
    // We trust the QC-signed block's app_hash as the anchor.
    let trusted_app_hash = block.app_hash;

    // 8. Update state to reflect the snapshot height
    *state.last_committed_height = snapshot.height;
    // Use the QC-verified app_hash, not the unverified snapshot.hash.
    *state.last_app_hash = trusted_app_hash;

    info!(height = snapshot.height.as_u64(), "snapshot sync complete");
    Ok(true)
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
    let mut pending_epoch: Option<Epoch> = None;

    for (i, (block, qc)) in blocks.iter().enumerate() {
        // H-7: Apply pending epoch transition at exactly start_view, matching
        // the engine's advance_view_to behavior.
        if let Some(ref ep) = pending_epoch
            && block.view >= ep.start_view
        {
            *state.current_epoch = pending_epoch.take().unwrap();
        }
        // Validate chain continuity
        if i > 0 && block.parent_hash != blocks[i - 1].0.hash {
            return Err(eg!(
                "chain discontinuity at height {}: expected parent {}, got {}",
                block.height.as_u64(),
                blocks[i - 1].0.hash,
                block.parent_hash
            ));
        }
        // F-06: Validate first block links to our last committed block
        if i == 0
            && state.last_committed_height.as_u64() > 0
            && let Some(last) = state
                .store
                .get_block_by_height(*state.last_committed_height)
            && block.parent_hash != last.hash
        {
            return Err(eg!(
                "sync batch first block parent {} does not match last committed block {} at height {}",
                block.parent_hash,
                last.hash,
                state.last_committed_height
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
            // Verify QC aggregate signature and quorum
            let verifier = hotmint_crypto::Ed25519Verifier;
            let qc_bytes = hotmint_types::vote::Vote::signing_bytes(
                state.chain_id_hash,
                cert.epoch,
                cert.view,
                &cert.block_hash,
                hotmint_types::vote::VoteType::Vote,
            );
            if !hotmint_types::Verifier::verify_aggregate(
                &verifier,
                &state.current_epoch.validator_set,
                &qc_bytes,
                &cert.aggregate_signature,
            ) {
                return Err(eg!(
                    "sync QC signature verification failed at height {}",
                    block.height.as_u64()
                ));
            }
            if !hotmint_crypto::has_quorum(
                &state.current_epoch.validator_set,
                &cert.aggregate_signature,
            ) {
                return Err(eg!(
                    "sync QC below quorum threshold at height {}",
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

        // Skip already-committed blocks
        if block.height <= *state.last_committed_height {
            continue;
        }

        // Defense-in-depth: verify the proposer is the correct leader for this view.
        // The QC already proves 2f+1 honest validators accepted this block (and they
        // checked the proposer), but we re-check here to catch corrupted sync data.
        if let Some(expected_leader) = state
            .current_epoch
            .validator_set
            .leader_for_view(block.view)
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

        // Store the block and its commit QC (H-12: so the node can serve
        // commit proofs to other syncing peers and light clients).
        state.store.put_block(block.clone());
        if let Some(commit_qc) = qc {
            state.store.put_commit_qc(block.height, commit_qc.clone());
        }

        // Run application lifecycle
        let ctx = BlockContext {
            height: block.height,
            view: block.view,
            proposer: block.proposer,
            epoch: state.current_epoch.number,
            epoch_start_view: state.current_epoch.start_view,
            validator_set: &state.current_epoch.validator_set,
            timestamp: block.timestamp,
            vote_extensions: vec![],
        };

        if !state.app.validate_block(block, &ctx) {
            return Err(eg!(
                "app rejected synced block at height {}",
                block.height.as_u64()
            ));
        }

        let txs = commit::decode_payload(&block.payload);
        let response = state
            .app
            .execute_block(&txs, &ctx)
            .c(d!("execute_block failed during sync"))?;

        state
            .app
            .on_commit(block, &ctx)
            .c(d!("on_commit failed during sync"))?;

        *state.last_app_hash = if state.app.tracks_app_hash() {
            response.app_hash
        } else {
            // App does not compute state roots: carry the chain's authoritative
            // value forward so that the continuity check stays coherent.
            block.app_hash
        };

        // Handle epoch transitions — defer to start_view (H-7)
        if !response.validator_updates.is_empty() {
            let new_vs = state
                .current_epoch
                .validator_set
                .apply_updates(&response.validator_updates);
            let epoch_start = ViewNumber(block.view.as_u64() + 2);
            pending_epoch = Some(Epoch::new(
                state.current_epoch.number.next(),
                epoch_start,
                new_vs,
            ));
        }

        *state.last_committed_height = block.height;
    }

    // If the pending epoch's start_view was reached by the last block, apply it.
    // Otherwise, return it so the caller (engine) can defer activation correctly.
    if let Some(ref ep) = pending_epoch
        && let Some((last_block, _)) = blocks.last()
        && last_block.view >= ep.start_view
    {
        *state.current_epoch = pending_epoch.take().unwrap();
    }

    // Return any still-pending epoch for the caller to handle.
    Ok(pending_epoch)
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
        let vote_bytes = hotmint_types::vote::Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            block.view,
            &block.hash,
            hotmint_types::vote::VoteType::Vote,
        );
        let sig = hotmint_types::Signer::sign(signer, &vote_bytes);
        let mut agg = hotmint_types::AggregateSignature::new(1);
        agg.add(0, sig).unwrap();
        QuorumCertificate {
            block_hash: block.hash,
            view: block.view,
            aggregate_signature: agg,
            epoch: EpochNumber(0),
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
        };
        assert!(replay_blocks(&blocks, &mut state).is_err());
    }
}
