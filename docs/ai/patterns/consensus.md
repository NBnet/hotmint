# Consensus Subsystem Review Patterns

## Files
- `crates/hotmint-consensus/src/engine.rs` (~2.3K LOC) — main engine, message dispatch
- `crates/hotmint-consensus/src/view_protocol.rs` — view step sequencing
- `crates/hotmint-consensus/src/state.rs` — current view, locked QC, epoch
- `crates/hotmint-consensus/src/vote_collector.rs` — quorum aggregation
- `crates/hotmint-consensus/src/commit.rs` — DC detection, ancestor chain commit
- `crates/hotmint-consensus/src/pacemaker.rs` — timeout scheduling
- `crates/hotmint-consensus/src/sync.rs` — state sync, block catch-up

## Architecture
- HotStuff-2 two-chain commit: QC (first cert) + DC (double cert) = finality
- View steps: Enter → Propose → Vote → Prepare → Vote2 → Commit
- 5 pluggable traits: Application, BlockStore, NetworkSink, Signer, Verifier
- Tokio select! loop in engine for concurrent message/timeout/sync handling

## Critical Invariants

### INV-CS1: Voting Safety Rule
A node votes for block B at view V only if:
1. B extends the node's locked block, OR
2. B carries a QC with view > node's locked_view
**Check**: Verify in view_protocol.rs that the vote decision checks BOTH conditions.

### INV-CS2: Quorum by Voting Power
QCs and TCs require signers holding strictly more than 2/3 of the current epoch's voting power: `ValidatorSet::quorum_threshold() = floor(total_power * 2 / 3) + 1` (crates/hotmint-types/src/validator.rs:168), enforced via `hotmint_crypto::aggregate::has_quorum` (crypto/src/aggregate.rs:21). For equal-power sets this is 2f+1 only when n ≡ 1 (mod 3), so state the power-based rule rather than a signature count.
**Check**: Verify vote_collector.rs derives its threshold from `quorum_threshold()`/`has_quorum` rather than a hard-coded signature count. Verify dedup by validator_id.

### INV-CS3: Double Certificate Validity
A DC is valid only if: both QCs reference the same block hash AND the same view (`outer_qc.view == inner_qc.view` — NOT consecutive views), the epochs are equal, the inner QC aggregates `VoteType::Vote` and the outer aggregates `VoteType::Vote2`, and both aggregates reach quorum.
**Check**: Verify DC validation in `engine.rs::validate_double_cert` checks all of these conditions. (commit.rs trusts the DC — it performs no cryptographic checks.)

### INV-CS4: Commit Completeness
When a DC is formed for block at height H, ALL blocks from last_committed_height+1 through H must be committed in order.
**Check**: Verify commit.rs walks the ancestor chain and commits each block sequentially.

### INV-CS5: View Monotonicity
A node's current view must never decrease (except on crash recovery to last persisted view).
**Check**: Verify all view advancement paths use `max(current_view, new_view)` or equivalent.

### INV-CS6: Pacemaker Guarantees
1. Timeout fires at least once per view (liveness)
2. Timeout duration increases on consecutive timeouts (exponential backoff)
3. Timeout resets on successful view change (not on proposal alone)
**Check**: Verify pacemaker.rs implements all three.

### INV-CS7: Message Epoch/View Validation
Every inbound consensus message must be validated for (epoch, view) before processing. Messages from wrong epoch/view must be dropped.
**Check**: Verify engine.rs checks message epoch and view before dispatching to view_protocol.

## Common Bug Patterns

### Locked QC Bypass (technical-patterns.md 1.2)
Vote cast for a block that conflicts with locked block and doesn't carry a higher QC.
**Trigger**: Receive proposal with QC.view < locked_view, vote anyway.

### QC With Insufficient Signers (technical-patterns.md 3.2)
Duplicate vote counted, forming a QC whose aggregate covers less power than a real quorum.
**Trigger**: Same validator sends vote twice (replay or honest resend), both counted.

### Pacemaker Never Fires (technical-patterns.md 2.1)
Timer scheduled but tokio::select! never picks the timeout branch because another branch always wins.
**Check**: Verify timeout is a dedicated select! branch, not hidden behind a condition.

## Review Checklist
- [ ] Voting rule checks locked_view AND extending chain
- [ ] Quorum = `floor(total_power * 2 / 3) + 1`, deduped by validator_id
- [ ] DC: same block hash, same view, Vote/Vote2 types, both aggregates at quorum (engine.rs)
- [ ] Commit walks full ancestor chain from DC block to last committed
- [ ] View number never decreases
- [ ] Pacemaker timeout fires reliably in tokio::select! loop
- [ ] Inbound messages checked for epoch and view before processing
- [ ] State persisted before commit response returned
- [ ] parking_lot locks NOT held across .await points
