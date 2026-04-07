# Storage Subsystem Review Patterns

## Files
- `crates/hotmint-storage/src/lib.rs` — VsdbBlockStore, StatePersistence, WAL, evidence store

## Architecture
- VsdbBlockStore wraps vsdb collections for block persistence
- Blocks indexed by hash and by height
- StatePersistence: consensus state (view, locked_qc, app_hash, epoch)
- Wal trait: commit intent logging with fsync
- Evidence store: equivocation proof tracking

## Critical Invariants

### INV-ST1: Crash Atomicity
Consensus state (current_view, locked_qc) must be persisted BEFORE the commit response is returned to the consensus engine.
**Check**: Verify state persistence is synchronous and fsynced, not deferred.

### INV-ST2: Block Store Consistency
A block stored by hash must be retrievable by that hash. A block stored at height H must be the only block at height H.
**Check**: Verify put-by-hash and put-by-height are atomic. Verify no duplicate heights.

### INV-ST3: WAL Durability
The commit intent WAL must be fsynced before the application's execute_block/on_commit is called.
**Check**: Verify WAL write + fsync ordering.

### INV-ST4: Evidence Persistence
Equivocation evidence must survive crashes.
**Check**: Verify evidence is fsynced before slashing action.

### INV-ST5: Recovery Correctness
On restart, the node must resume from the last persisted state without re-voting for already-committed views.
**Check**: Verify recovery reads persisted state and sets current_view, locked_qc correctly.

## Common Bug Patterns

### State Persistence Gap (technical-patterns.md 6.1)
Commit returns before state is persisted. Crash → re-vote → safety violation.

### Block Store / App State Divergence (technical-patterns.md 6.2)
Block applied to app but not stored in BlockStore. Restart → block missing from chain.

## Review Checklist
- [ ] State persisted (fsynced) before commit returns
- [ ] Block stored before application commit
- [ ] No duplicate blocks at same height
- [ ] WAL fsynced before application execution
- [ ] Evidence persisted before slashing
- [ ] Recovery sets correct view and locked_qc
- [ ] RwLock on SharedBlockStore: read-heavy, write-rare pattern verified
