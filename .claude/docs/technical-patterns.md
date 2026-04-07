# Hotmint Technical Bug Patterns

This document catalogs known bug categories in BFT consensus engines.
Load this document FIRST before performing any review or debug analysis.

---

## Category 1: Safety Violations (Agreement)

### 1.1 Equivocation Not Detected
**Pattern**: A Byzantine validator signs two different blocks at the same (view, height). The system fails to detect or record the equivocation evidence.
**Where**: `consensus/src/view_protocol.rs` — vote processing, `consensus/src/engine.rs` — evidence handling.
**Impact**: Safety violation — two conflicting blocks may both be committed.
**Check**: Verify every received vote is checked against previously stored votes for the same (validator, view). Verify evidence is persisted.

### 1.2 Locked QC Bypass
**Pattern**: A node votes for a block B2 at view V2 even though it has a locked QC for block B1 at view V1 < V2, and B2 does not extend B1.
**Where**: `consensus/src/view_protocol.rs` — vote decision logic.
**Impact**: Safety violation — conflicting blocks can both achieve QCs.
**Check**: Verify the voting rule: a node may only vote for a block that (1) extends the locked block, OR (2) has a QC with view > locked_view.

### 1.3 Double Certificate Forgery
**Pattern**: A DoubleCertificate (DC) is accepted without verifying that both QCs are for the same block at consecutive views.
**Where**: `consensus/src/commit.rs` — DC validation.
**Impact**: Premature commit of an unconfirmed block.
**Check**: Verify DC validation checks: same block hash, view_qc2 == view_qc1 + 1, both QCs have valid 2f+1 signatures.

### 1.4 Commit Without Ancestor Chain
**Pattern**: A block is committed but its ancestors (back to the last committed block) are not committed first.
**Where**: `consensus/src/commit.rs` — commit chain walk.
**Impact**: Gap in committed blockchain — missing blocks.
**Check**: Verify commit walks the ancestor chain from the DC block back to the last committed height, committing each block in order.

---

## Category 2: Liveness Violations

### 2.1 Pacemaker Stuck
**Pattern**: The pacemaker timeout never fires or fires with wrong duration, causing the node to stay in a view forever.
**Where**: `consensus/src/pacemaker.rs` — timeout scheduling.
**Impact**: Liveness failure — no new views, no new blocks.
**Check**: Verify timeout is scheduled on view entry. Verify exponential backoff formula: `base * 1.5^(round)` capped at `max_timeout`. Verify timer is reset on view change.

### 2.2 View Advancement Deadlock
**Pattern**: All honest nodes are waiting for a message from the leader, but the leader is waiting for votes from the previous view that will never arrive.
**Where**: `consensus/src/view_protocol.rs` — view enter logic, `pacemaker.rs`.
**Impact**: Cluster hangs until timeout cascades.
**Check**: Verify TimeoutCertificate (TC) handling allows view advancement without the leader's proposal. Verify TC requires 2f+1 timeout messages.

### 2.3 Sync Loop
**Pattern**: A node falls behind (e.g., after restart) and the sync protocol enters an infinite loop requesting the same blocks repeatedly.
**Where**: `consensus/src/sync.rs` — block fetch loop.
**Impact**: Node never catches up, high network traffic.
**Check**: Verify sync advances its "from_height" cursor after each batch. Verify termination condition when caught up.

---

## Category 3: Vote Collection Bugs

### 3.1 Threshold Calculation Error
**Pattern**: The 2f+1 quorum threshold is computed incorrectly. For n=3f+1 validators, quorum should be 2f+1 = (2n+1)/3 rounded up.
**Where**: `consensus/src/vote_collector.rs` — threshold computation.
**Impact**: QCs formed with insufficient votes (safety) or requiring too many votes (liveness).
**Check**: Verify `threshold = 2 * (n - 1) / 3 + 1` or equivalent for general n.

### 3.2 Duplicate Vote Counting
**Pattern**: The same validator's vote is counted multiple times toward the quorum.
**Where**: `consensus/src/vote_collector.rs` — vote insertion.
**Impact**: QC formed with fewer distinct signers than required.
**Check**: Verify deduplication by validator_id before counting. Verify the set of signers is checked against the validator set.

### 3.3 Vote From Wrong Epoch/View
**Pattern**: A vote for view V is accepted and counted toward the QC for view V' != V, or a vote from epoch E is counted in epoch E'.
**Where**: `consensus/src/vote_collector.rs` — vote validation.
**Impact**: QC contains votes for different views — invalid.
**Check**: Verify vote's (epoch, view, block_hash) matches the collector's target before accepting.

### 3.4 Stale QC Accepted
**Pattern**: A QC from a past view is accepted as if it were from the current view.
**Where**: `consensus/src/view_protocol.rs` — message handling.
**Impact**: State machine confusion — may trigger incorrect state transitions.
**Check**: Verify QC view is checked against the current view before using it to advance state.

---

## Category 4: Epoch Transition Bugs

### 4.1 Validator Set Applied Too Early
**Pattern**: A validator set change takes effect immediately instead of at commit_view + 2, causing the validating set to disagree between nodes that have and haven't processed the transition.
**Where**: `consensus/src/state.rs` — epoch transition, `types/src/` — Epoch type.
**Impact**: Fork — nodes disagree on which validators are valid for a given view.
**Check**: Verify validator set changes are buffered and applied exactly at the specified future view.

### 4.2 Cross-Epoch Signature Verification
**Pattern**: A message signed by a validator in epoch E is rejected because the verifier only checks the epoch E+1 validator set.
**Where**: `consensus/src/view_protocol.rs` — signature verification.
**Impact**: Liveness — valid messages from the old epoch are dropped during transition.
**Check**: Verify the previous epoch's validator set is retained for message verification during the transition window.

### 4.3 Epoch Boundary Double Commit
**Pattern**: The last block of epoch E and the first block of epoch E+1 both trigger validator set updates, causing a double transition.
**Where**: `consensus/src/commit.rs` — epoch transition on commit.
**Impact**: Validator set corrupted — wrong set active.
**Check**: Verify epoch transition is idempotent or guarded against duplicate application.

---

## Category 5: Networking & Message Handling Bugs

### 5.1 Message Replay Attack
**Pattern**: A Byzantine node replays a valid old message (e.g., a vote from view V-10). The receiver processes it as current.
**Where**: `network/src/service.rs` — inbound message handling, `consensus/src/engine.rs`.
**Impact**: Vote counting pollution, wasted resources.
**Check**: Verify inbound messages are checked against current (epoch, view) before processing. Stale messages should be dropped.

### 5.2 Unbounded Message Queue
**Pattern**: The `mpsc::channel` from network to consensus has no backpressure, or its capacity is too large.
**Where**: `consensus/src/engine.rs` — channel creation, `network/src/service.rs` — send path.
**Impact**: Memory exhaustion under message flood (DoS).
**Check**: Verify channel has bounded capacity. Verify sender handles `TrySendError::Full` gracefully (drop or backpressure).

### 5.3 Missing Message Authentication
**Pattern**: A consensus message is processed without first verifying its signature.
**Where**: `consensus/src/engine.rs` — message dispatch.
**Impact**: Byzantine node can inject arbitrary messages without being a valid signer.
**Check**: Verify every inbound consensus message has its signature verified against the current (or previous) validator set BEFORE any state mutation.

### 5.4 Peer Map Inconsistency
**Pattern**: `PeerMap` (ValidatorId ↔ PeerId) becomes inconsistent — the forward map says A→P1 but the reverse says P1→B.
**Where**: `network/src/` — peer management.
**Impact**: Messages routed to wrong validator.
**Check**: Verify all insert/remove operations update both directions atomically.

---

## Category 6: Storage & Crash Recovery Bugs

### 6.1 State Persistence Gap
**Pattern**: The node commits a block to the application but crashes before persisting the updated consensus state (current_view, locked_qc, app_hash).
**Where**: `storage/` — StatePersistence, `consensus/src/commit.rs`.
**Impact**: On restart, the node re-enters a view it already committed, potentially voting for a conflicting block.
**Check**: Verify consensus state is persisted BEFORE the commit response is returned. Verify WAL is fsynced.

### 6.2 Block Store Missing Committed Block
**Pattern**: The commit path writes the block to the application but crashes before writing to the BlockStore.
**Where**: `storage/` — VsdbBlockStore, `consensus/src/commit.rs`.
**Impact**: On restart, the block is applied but not queryable. Subsequent commits may re-commit it.
**Check**: Verify block is persisted to BlockStore BEFORE application commit. Or: verify recovery reconciles the two.

### 6.3 Evidence Store Not Crash-Safe
**Pattern**: Equivocation evidence is detected but crashes before persisting. On restart, the evidence is lost.
**Where**: `storage/` — evidence persistence.
**Impact**: Byzantine validator escapes slashing.
**Check**: Verify evidence is fsynced to disk before any action is taken on it.

---

## Category 7: Cryptographic Bugs

### 7.1 Domain Separation Missing
**Pattern**: Vote signing does not include chain_id, epoch, or view in the signed message, enabling cross-chain or cross-epoch replay.
**Where**: `crypto/src/lib.rs` — sign/verify, `types/src/` — signable types.
**Impact**: A vote from chain A can be replayed on chain B.
**Check**: Verify signed payload includes: `chain_id_hash || epoch || view || message_type || block_hash`.

### 7.2 Batch Verification Short-Circuit
**Pattern**: Batch signature verification passes if ANY signature is valid, instead of requiring ALL to be valid.
**Where**: `light/src/lib.rs` — batch verification.
**Impact**: Light client accepts a header with forged signatures.
**Check**: Verify batch verification returns true only if ALL signatures in the batch are valid.

### 7.3 Signature Malleability
**Pattern**: Two different signature byte sequences verify for the same (public_key, message), allowing duplicate detection to be bypassed.
**Where**: `crypto/src/lib.rs` — ed25519 verification.
**Impact**: Same vote appears twice with different signatures, bypassing dedup.
**Check**: Verify ed25519-dalek is configured with strict verification (reject non-canonical signatures).
