# Audit Findings

> Auto-managed by /x-review and /x-fix.
> Historical audit rounds (formerly in `docs/security-audit-and-roadmap.md`) consolidated here.
> Earlier rounds (C-1..C-7, H-1..H-12, R-1, A-1..A-8, B-1..B-3, C2-1..C2-5) were resolved prior to Round 3 and are not itemized.

## Open

*(No open findings)*

---

## Won't Fix

### [HIGH] light: BlockHeader fields not cryptographically bound to QC-verified hash
- **Where**: crates/hotmint-light/src/lib.rs:87-93
- **What**: verify_header checks qc.block_hash == header.hash but never verifies header fields produce that hash. Block::compute_hash() mixes in payload and evidence which BlockHeader omits. An attacker with a valid QC can craft a BlockHeader with arbitrary field values (e.g. forged app_hash) and set header.hash to the real hash.
- **Reason**: Requires architectural redesign — either adding a fields_hash to Block/BlockHeader and restructuring compute_hash() as hash(header_fields_hash || payload_hash || evidence_hash), or carrying the full payload/evidence hashes in BlockHeader. Both approaches change the block serialization format and affect many subsystems. Tracked for a future protocol version.

### [LOW] crypto: verify_batch() does not perform small-order public key rejection unlike verify_strict()
- **Where**: crates/hotmint-crypto/src/signer.rs:64 vs :109
- **What**: Individual verification uses verify_strict() which rejects small-order public keys. Batch verification uses ed25519_dalek::verify_batch() which does not. A validator with one of the 8 small-order Curve25519 points could produce signatures passing batch but failing strict verification.
- **Reason**: ed25519_dalek's verify_batch API does not expose a strict mode. A workaround (loop of verify_strict) trades ~2× batch throughput. Practical risk is negligible: only 8 small-order points exist, and validator registration controls key acceptance. Tracked for upstream ed25519-dalek enhancement.

---

## Resolved — Round 3 (2026-04-07)

> **Scope:** Full codebase (~16K LOC), all crates
> **Findings:** 12 total (0 critical, 5 high, 5 medium, 2 low)

| Subsystem | Findings | Severity |
|-----------|:--------:|----------|
| Network | 2 | HIGH, MEDIUM |
| Storage | 3 | HIGH, HIGH, HIGH |
| API & ABCI | 3 | HIGH, MEDIUM, MEDIUM |
| Consensus | 1 | MEDIUM |
| Crypto | 1 | MEDIUM |
| Staking | 1 | LOW |
| Mgmt | 1 | LOW |

### [x] A3-1. PeerMap Bidirectional Consistency Bug `[HIGH — Network]`
- **Where:** `crates/hotmint-network/src/service.rs` — `PeerMap::insert`
- **What:** `insert(vid, new_pid)` updates forward map but does NOT remove the stale reverse mapping `peer_to_validator[old_pid] → vid`. After a validator reconnects with a new PeerId, messages from the old PeerId are misattributed.
- **Fix:** Remove old reverse mapping in `insert()`.

### [x] A3-2. Evidence Store Not Fsynced `[HIGH — Storage]`
- **Where:** `crates/hotmint-storage/src/evidence_store.rs` — `put_evidence`, `persist_next_id`
- **What:** `persist_next_id()` uses `std::fs::write()` without `sync_all()`. No `flush()` on the `EvidenceStore` trait. A crash loses equivocation evidence.
- **Fix:** Add `sync_all()` to `persist_next_id()`. Add `flush()` to trait. Call `flush()` from engine after `put_evidence()`.

### [x] A3-3. Block Store put_block Not Atomic `[HIGH — Storage]`
- **Where:** `crates/hotmint-storage/src/block_store.rs` — `put_block`
- **What:** Two separate vsdb inserts (`by_height`, then `by_hash`) are not atomic. Crash between them leaves dangling hash reference.
- **Fix:** Reverse order (insert `by_hash` first, then `by_height`).

### [x] A3-4. Evidence Not Flushed Before Consensus State Persist `[HIGH — Storage]`
- **Where:** `crates/hotmint-consensus/src/engine.rs` — `process_commit_result`
- **What:** `ev_store.mark_committed()` modifies vsdb but no flush follows before `persist_state()`. Crash loses evidence while consensus state survives.
- **Fix:** Add `ev_store.flush()` before `persist_state()`.

### [x] A3-5. WebSocket Connection Limit TOCTOU Race `[HIGH — API]`
- **Where:** `crates/hotmint-api/src/http_rpc.rs` — `ws_upgrade_handler`
- **What:** Check `ws_connection_count` then increment asynchronously — concurrent upgrades can exceed `MAX_WS_CONNECTIONS`.
- **Fix:** Use `compare_exchange` on AtomicU64, or acquire semaphore before accepting upgrade.

### [x] A3-6. Missing Double Certificate View Ordering Validation `[MEDIUM — Consensus]`
- **Where:** `crates/hotmint-consensus/src/engine.rs` — `validate_double_cert`
- **What:** Does not validate `outer_qc.view >= inner_qc.view`. Malformed DC with reversed view ordering could pass.
- **Fix:** Add view ordering check.

### [x] A3-7. Ed25519 Signature Malleability — Non-Canonical S Accepted `[MEDIUM — Crypto]`
- **Where:** `crates/hotmint-crypto/src/signer.rs` — `verify`
- **What:** ed25519-dalek default verification does not reject non-canonical signatures (S >= group order).
- **Fix:** Enable `strict_signatures` feature.

### [x] A3-8. Relay Dedup Truncates Blake3 Hash to 8 Bytes `[MEDIUM — Network]`
- **Where:** `crates/hotmint-network/src/service.rs` — relay deduplication
- **What:** Only first 8 bytes of blake3 (64-bit) used for dedup. Birthday-bound collisions drop legitimate messages.
- **Fix:** Use full 32-byte hash.

### [x] A3-9. Silent Hash Truncation/Padding in ABCI Protobuf Deserialization `[MEDIUM — ABCI]`
- **Where:** `crates/hotmint-abci-proto/src/convert.rs` — `bytes_to_hash`
- **What:** Silently truncates or zero-pads hash fields that aren't exactly 32 bytes, corrupting cryptographic integrity.
- **Fix:** Return `Err` instead of silently padding.

### [x] A3-10. Application Error Messages Exposed in RPC Responses `[MEDIUM — API]`
- **Where:** `crates/hotmint-api/src/rpc.rs`
- **What:** Internal errors forwarded verbatim to untrusted clients — information leakage.
- **Fix:** Return generic error messages to clients; log detailed errors server-side only.

### [x] A3-11. ValidatorId Not Derived From Public Key `[LOW — Staking]`
- **Where:** `crates/hotmint/src/bin/node.rs` — ValidatorId lookup
- **What:** ValidatorId assigned manually via flag, not derived from public key hash.
- **Fix:** Derive ValidatorId from public key hash at registration time.

### [x] A3-12. Missing SAFETY Comments on Unsafe Blocks `[LOW — Mgmt]`
- **Where:** `crates/hotmint-mgmt/src/local.rs` — two `libc::kill` blocks
- **What:** Both `unsafe` blocks lack `// SAFETY:` documentation.
- **Fix:** Add `// SAFETY:` comments.

### Round 3 — Clean Areas

| Subsystem | Verified |
|-----------|----------|
| Consensus core | INV-CS1 (voting safety), INV-CS2 (2f+1 quorum), INV-CS4 (commit completeness), INV-CS5 (view monotonicity), INV-CS7 (epoch/view validation); vote dedup; equivocation detection |
| Pacemaker | INV-CS6 (timeout fires per view, exponential backoff 1.5x capped 30s, reset on view change) |
| Sync | Cursor advances; terminates when caught up; no infinite loop; no consensus interference |
| Mempool | INV-MP1 (ordering), INV-MP2 (no duplicates), INV-MP3 (eviction), INV-MP4 (RBF atomicity); lock ordering correct |
| Light client | INV-CR3 (batch all-or-nothing); height monotonicity; hash chain verification |
| Domain separation | INV-CR1 — all 5 message types include chain_id, epoch, view, type tag |
| Epoch transitions | Atomic; +2 view delay; slashing verified; unbonding prevents slash evasion |
| ABCI framing | INV-API3 — 64MB bound; length validated before allocation |
| API read-only | INV-API1 — no write locks in RPC handlers |
| Concurrency | No parking_lot across .await; all channels bounded; select! cancel-safe |

---

## Resolved — Round 4 (2026-04-12)

> **Scope:** Full codebase (~16K LOC), all crates
> **Base Commit:** `4044a24` (v0.8.6)
> **Findings:** 8 total (0 critical, 1 high, 0 medium, 7 low)

| Subsystem | Findings | Severity |
|-----------|:--------:|----------|
| Sync | 1 | HIGH |
| Engine | 1 | LOW |
| Network | 4 | LOW, LOW, LOW, LOW |
| ABCI Proto | 1 | LOW |
| Facade & Mgmt | 1 | LOW (style) |

### [x] A4-1. `replay_blocks` Drops Pending Epoch From Prior Batch — Cross-Epoch Sync Broken `[HIGH — Sync]`
- **Where:** `crates/hotmint-consensus/src/sync.rs:428`
- **What:** `replay_blocks()` initializes local `pending_epoch` to `None` instead of reading from `state.pending_epoch`. Cross-batch epoch transitions verify against the OLD validator set, causing sync abort.
- **Fix:** `let mut pending_epoch: Option<Epoch> = state.pending_epoch.take();`

### [x] A4-2. Equivocation Evidence Not Flushed Immediately After Detection `[LOW — Engine]`
- **Where:** `crates/hotmint-consensus/src/engine.rs:1340-1354`
- **What:** `handle_equivocation()` calls `put_evidence()` but never `flush()`. Crash before next commit loses evidence. (Detection-path gap remaining after A3-2/A3-4 fixes.)
- **Fix:** Add `evidence_store.flush()` after `put_evidence` in `handle_equivocation()`.

### [x] A4-3. PeerMap.insert Does Not Clean Stale Reverse Mapping When PeerId Is Reused `[LOW — Network]`
- **Where:** `crates/hotmint-network/src/service.rs:67-72`
- **What:** When `pid` already maps to a different ValidatorId, the old forward entry is left dangling. (Symmetric reverse-direction case of A3-1.)
- **Fix:** Clean old forward mapping when reverse mapping is overwritten.

### [x] A4-4. Eviction Does Not Clean Mempool Peer Tracking `[LOW — Network]`
- **Where:** `crates/hotmint-network/src/service.rs:882-886`
- **What:** Evicted peers not removed from `mempool_notif_connected_peers` or `mempool_peer_rate`.
- **Fix:** Remove evicted peer from mempool tracking maps.

### [x] A4-5. ConnectionClosed Does Not Clean Mempool Peer Tracking `[LOW — Network]`
- **Where:** `crates/hotmint-network/src/service.rs:902-913`
- **What:** Same as A4-4 but for normal TCP disconnects. Consensus peers cleaned, mempool peers not.
- **Fix:** Add mempool cleanup to ConnectionClosed handler.

### [x] A4-6. Relay Broadcasts to `connected_peers` Instead of `notif_connected_peers` `[LOW — Network]`
- **Where:** `crates/hotmint-network/src/service.rs:519`
- **What:** Relay iterates all TCP-connected peers rather than those with open notification substreams, generating unnecessary failed sends.
- **Fix:** Use `notif_connected_peers` for relay path.

### [x] A4-7. `assert!` Panic in `bytes_to_hash` on Malformed ABCI Protobuf Input `[LOW — ABCI Proto]`
- **Where:** `crates/hotmint-abci-proto/src/convert.rs:324-328`
- **What:** `assert!(bytes.len() == 32)` panics on non-32-byte non-empty input from ABCI app. (Remaining case after A3-9 fix.)
- **Fix:** Replace `assert!` with fallible conversion returning `Result`.

### [x] A4-8. Inline-Path Rule Violations Across 3 Files `[LOW — Style]`
- **Where:** `crates/hotmint-mgmt/src/lib.rs`, `crates/hotmint/src/bin/node.rs`, `crates/hotmint/src/config.rs`
- **What:** Multiple files use fully-qualified paths 5-11 times without a top-level `use` import.
- **Fix:** Add appropriate `use` imports.

### Round 4 — Clean Areas

| Subsystem | Verified |
|-----------|----------|
| Consensus core | INV-CS1..CS5, CS7; vote dedup; equivocation detection |
| Pacemaker | INV-CS6 (timeout, backoff `1.5^n` capped, reset on DC/TC) |
| Types & crypto | INV-CR1 (domain separation), INV-CR2 (`verify_strict`), INV-CR3 (batch all-or-nothing), INV-CR4 (no Blake3 truncation), INV-CR5 (ValidatorId deterministic) |
| Storage | INV-ST1 (WAL fsync), INV-ST2 (by_hash before by_height), INV-ST3 (WAL before app commit), INV-ST5 (recovery reads persisted state) |
| Mempool | INV-MP1..MP4; lock ordering `entries` before `seen`; pool size accurate |
| API | INV-API1 (no writes), INV-API2 (rate limiting), INV-API4 (WS RAII guard); TCP limit 256; 1MB line; 30s timeout |
| ABCI | INV-API3 (64MB cap on all paths); no sensitive data in errors |
| Light client | INV-CR3 (batch soundness); quorum + aggregate sig + height monotonicity |
| Staking | `checked_add`/`saturating_sub`; evidence verified before slash; unbonding correct; epoch +2 delay |
| Engine | Epoch/view validation; sig before mutation; 100 msg/s rate limit; no lock across await; cancel-safe select! |
| Unsafe blocks | Both `libc::kill`: SAFETY comments present; PID validated |
| Concurrency | No parking_lot across .await; all consensus channels bounded; cancel-safe select!; consistent lock ordering |

---

## References

- [HotStuff-2 Paper](https://arxiv.org/abs/2301.03253)
- [CometBFT v0.38 Documentation](https://docs.cometbft.com/v0.38/introduction/)
- [CometBFT ABCI++ Specification](https://docs.cometbft.com/v0.38/spec/abci/)
