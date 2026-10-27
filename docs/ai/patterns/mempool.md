# Mempool Subsystem Review Patterns

## Files
- `crates/hotmint-mempool/src/lib.rs` — Mempool struct, MempoolAdapter trait

## Architecture
- BTreeSet ordered by (priority ASC, tx_hash ASC) — highest priority = last
- HashMap<TxHash, u64> for dedup and priority tracking
- Replace-by-Fee: same tx bytes with higher priority replaces old entry
- Eviction: when full, new tx must have priority > min entry
- MempoolAdapter trait for pluggable implementations

## Critical Invariants

### INV-MP1: Ordering Consistency
BTreeSet ordering and HashMap priority must agree. If the set has entry (p=5, hash=H), the map must have H→5.
**Check**: Verify both structures are updated atomically on insert/remove/replace.

### INV-MP2: No Duplicate Transactions
A transaction hash must appear at most once in the pool.
**Check**: Verify insert checks HashMap before adding. Verify RBF removes old entry before inserting new.

### INV-MP3: Eviction Correctness
Eviction removes the LOWEST priority entry (first in BTreeSet). After eviction, pool size decreases by exactly 1.
**Check**: Verify eviction removes from BOTH BTreeSet and HashMap.

### INV-MP4: RBF Atomicity
Replace-by-Fee must remove the old entry and insert the new one atomically. If the remove succeeds but insert fails, the tx is lost.
**Check**: Verify old entry is removed only after new entry is confirmed insertable.

## Common Bug Patterns

### HashMap/BTreeSet Desync
Remove from BTreeSet but forget to remove from HashMap (or vice versa). Causes ghost entries.
**Trigger**: Error path returns early after partial removal.

### Priority Inversion
BTreeSet uses (priority, hash) but some code compares by hash only, losing priority ordering.

## Review Checklist
- [ ] BTreeSet and HashMap always updated in sync
- [ ] No duplicate tx_hash in pool
- [ ] Eviction removes lowest-priority entry from both structures
- [ ] RBF: old entry removed, new inserted, no intermediate state visible
- [ ] Lock ordering: entries lock before seen lock (if separate)
- [ ] Pool size accurately tracked
