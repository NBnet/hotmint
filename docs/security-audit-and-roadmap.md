# Hotmint Security Audit & Evolution Roadmap

> **Report Version:** Based on Hotmint v0.8 / CometBFT v0.38
> **Generated:** 2026-03-24 | **Last Audit:** 2026-03-25 | **Last Document Sync:** 2026-03-25
> **Sources:** CometBFT feature gap analysis + two rounds of code security audit
> **Purpose:** Serves as a reference baseline for the long-term evolution roadmap. Update completion status after each iteration (change `[ ]` to `[x]`, partially complete marked `[~]`).

---

## 1. Executive Summary

| Dimension | CometBFT v0.38 | Hotmint v0.8 |
|-----------|---------------|-------------|
| Language | Go | Rust |
| Consensus Algorithm | Tendermint (three-phase BFT) | HotStuff-2 (two-chain commit BFT) |
| Maturity | Production-grade, primary engine of Cosmos ecosystem | Architecturally complete, core feature parity achieved, entering production hardening phase |
| Core Strengths | Complete ecosystem, rich toolchain, mature protocol | Lower latency, more modular architecture, memory safety |
| Main Weaknesses | Three-phase voting latency, Go GC tail-latency jitter | Missing IBC cross-chain protocol; ecosystem tooling still maturing |

Hotmint's combination of **Rust + HotStuff-2 + litep2p** gives it the potential to surpass CometBFT in core consensus algorithm and architectural modernization. All original security vulnerabilities (C-1..C-7) and engineering defects (H-1..H-12, R-1) have been resolved. Core feature parity with CometBFT has been largely achieved — the remaining gaps are concentrated in:
- **Ecosystem Expansion Layer:** IBC cross-chain protocol (infrastructure ready, protocol not implemented)
- **Production Hardening:** Operations CLI completeness

---

## 2. Core Consensus Protocol Comparison

### 2.1 Algorithm Layer

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Protocol Family | Tendermint BFT | HotStuff-2 (arXiv:2301.03253) |
| Voting Phases | Three-phase: Propose → Pre-vote → Pre-commit | Two-chain: Propose → Vote → QC → Vote2 → DC |
| Commit Rule | Single-block commit after Pre-commit exceeds 2/3 | Double Certificate (two rounds of 2f+1) two-chain commit |
| View Change | Complex: requires collecting prevotes, has Nil vote path | Linear: Wish messages aggregate into TimeoutCert, no extra overhead |
| Proposer Election | Weighted round-robin | Weighted round-robin (`view % total_power` cumulative weight) ✅ |
| Network Complexity | O(n²) broadcast | O(n²) (same order, but fewer phases) |
| Theoretical Latency | ~2 network round-trips (three phases) | ~2 network round-trips (two phases, each with QC aggregation) |
| BFT Fault Tolerance | f < n/3 | f < n/3 |
| Timestamp Source | BFT Time (median of validator vote timestamps) | Proposer-specified with monotonicity validation + `MAX_FUTURE_DRIFT_MS` check ✅ |

### 2.2 Security Mechanisms

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Replay Attack Protection | Chain ID encoded in signature domain | Blake3(chain_id) injected into all signatures ✅ |
| State Fork Detection | App hash chain + ABCI verification | App hash chain (each block header carries previous block's execution result) ✅ |
| Double-Signing Detection | Complete evidence collection + network broadcast | Detection + P2P broadcast + vsdb persistent storage + signature verification ✅ |
| WAL Crash Recovery | Has Write-Ahead Log, precise replay | `ConsensusWal` two-phase commit (CommitIntent/CommitDone) + crash recovery ✅ |
| Locking Mechanism | polkaValue / round lock | `locked_qc` (safety equivalent) ✅ |
| Cross-Epoch Vote Replay Protection | Epoch encoded in signature or state machine transition protection | `signing_bytes` contains `chain_id_hash + epoch + view + block_hash` (HOTMINT_VOTE_V2) ✅ |

---

## 3. Application Interface (ABCI Layer) Comparison

### 3.1 Full Interface Method Comparison

| Method/Callback | CometBFT ABCI++ v0.38 | Hotmint `Application` Trait | Status |
|-----------------|----------------------|----------------------------|--------|
| Block Proposal Construction | `PrepareProposal` | `create_payload` | ✅ Semantically equivalent |
| Block Proposal Validation | `ProcessProposal` | `validate_block` | ✅ Semantically equivalent |
| Transaction Execution | `FinalizeBlock` | `execute_block` | ✅ Semantically equivalent |
| Transaction Pre-validation | `CheckTx` | `validate_tx` | ✅ Semantically equivalent |
| Block Commit Callback | `Commit` (includes app_hash) | `on_commit` | ✅ |
| Evidence Punishment | `FinalizeBlock.misbehavior[]` | `on_evidence(EquivocationProof)` | ✅ |
| State Query | `Query` | `query(path, data)` | ✅ |
| **Vote Extension Attachment** | **`ExtendVote`** | `extend_vote` | ✅ |
| **Vote Extension Verification** | **`VerifyVoteExtension`** | `verify_vote_extension` | ✅ |
| Snapshot Creation | `ListSnapshots` / `LoadSnapshotChunk` | `list_snapshots` / `load_snapshot_chunk` | ✅ |
| Snapshot Reception | `OfferSnapshot` / `ApplySnapshotChunk` | `offer_snapshot` / `apply_snapshot_chunk` | ✅ |
| Application Info | `Info` (includes last_block_height) | `info()` → `AppInfo { last_block_height, last_block_app_hash }` | ✅ |
| Genesis Initialization | `InitChain` | `init_chain(app_state: &[u8]) → BlockHash` | ✅ |

### 3.2 Cross-Process Communication

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| In-Process Interface | Go interface | Rust trait ✅ |
| Cross-Language/Cross-Process | gRPC (`.proto` multi-language SDK) | Unix domain socket + protobuf (`hotmint-abci`) + Go SDK |
| IPC Timeout Protection | gRPC built-in timeout | 5s read/write timeout (`set_read_timeout` / `set_write_timeout`) ✅ |

---

## 4. P2P Network Layer Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Underlying Framework | Custom MConnTransport (multiplexed TCP) | litep2p (Rust, derived from Polkadot ecosystem) ✅ |
| Message Routing | Reactor model | Notification + Request-Response protocol separation ✅ |
| Peer-to-Peer Encryption | SecretConnection (Noise) | litep2p built-in Noise/TLS ✅ |
| Peer Discovery | PEX Reactor + seed nodes | PEX protocol (`/hotmint/pex/1`) ✅ |
| Validator Connection Protection | Persistent Peers with priority reserved slots | Validators bypass max_peers limit ✅ (dedicated reserved slots pending) |
| Connection Management | Persistent/non-persistent peers + dial scheduling | Maintenance loop (10s) + backoff + chain_id handshake isolation ✅ |
| Message Compression | Internal protocol handling | zstd compression, compression-side Result propagation ✅ |

---

## 5. Mempool Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Data Structure | Concurrent linked list + LRU dedup cache | `BTreeSet<TxEntry>` + `HashMap<TxHash, u64>` ✅ |
| Ordering Strategy | **Priority queue** (application returns priority field) | Priority queue (priority ASC + hash tiebreak) + RBF ✅ |
| Capacity Control | `max_txs` (count) + `max_txs_bytes` (total bytes) | `max_size` (count) + `max_tx_bytes` (per-tx) |
| Overflow Eviction | Low-priority transactions evicted | Low-priority eviction ✅ |
| Re-validation | Re-runs `CheckTx` on pending txs after block production | `recheck()` async re-validation after each commit, evicts invalid txs ✅ |
| Gas Awareness | Application returns `gas_wanted`, Mempool evicts accordingly | `gas_wanted` field + `max_gas_per_block` truncation ✅ |
| API Rate Limiting | Supports rate limiting configuration | Token bucket rate limiting (TCP per-conn + HTTP global) ✅ |
| P2P Broadcast | Flood Mempool, peer Gossip | `tx_gossip` channel broadcasts accepted txs to peers ✅ |

---

## 6. Block Sync Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Implementation | Block Sync Reactor, concurrent multi-node download | Pipelined single-node: prefetch next batch while replaying current (max 100 blocks/batch) ✅ |
| Verification Strength | Per-block commit signature verification (2/3+) | Chain continuity + QC signature/quorum verification + app_hash consistency ✅ |
| Post-Catchup Switch | Automatic switch to consensus reactor | Starts consensus engine after `sync_to_tip` completes ✅ |

---

## 7. State Sync Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Capability | **Full support:** snapshot listing, chunked download, verification, application | Full support (`sync_via_snapshot` + chunked download) ✅ |
| Application-Side Interface | `ListSnapshots`, `LoadSnapshotChunk`, `OfferSnapshot`, `ApplySnapshotChunk` | All 4 methods implemented ✅ |
| Typical Join Time | Minutes (snapshot download) | Minutes (snapshot) or proportional to chain age (full replay) |

---

## 8. Light Client Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Implementation | Complete: bisection verification, untrusted range skipping | `hotmint-light` crate: `verify_header` + `update_validator_set` ✅ |
| Merkle Proof Output | `Query` returns Merkle proof | MPT state proof via vsdb `MptProof` + RPC `get_header` / `get_commit_qc` ✅ |
| Cross-Chain Foundation | IBC protocol depends on light client | Light client + Merkle proof infrastructure ready; IBC protocol not yet implemented |

---

## 9. RPC / API Layer Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Transport Protocol | HTTP + WebSocket (standard) | TCP JSON + axum HTTP/WS (`POST /` + `GET /ws`) ✅ |
| Event Subscription | WebSocket `subscribe` (rich filter syntax) | WS push with `SubscribeFilter` (event_types / height range / tx_hash) ✅ |
| Method Count | 20+ methods | 10+ methods (status, block, epoch, peers, submit_tx, header, commit_qc, get_tx, etc.) |
| Transaction Query | Query by hash, event indexing | `get_tx` by hash (vsdb tx_index: hash → height + index) ✅ |

---

## 10. Observability & Operations Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Prometheus Metrics | Rich (consensus round, P2P traffic, Mempool depth, etc.) | Basic (view, height, blocks, votes, timeouts) ✅ |
| Structured Logging | slog/zap | `tracing` crate ✅ |
| WAL Crash Recovery | Has WAL, precise recovery to pre-crash vote state | `ConsensusWal` (CommitIntent/CommitDone two-phase, crash recovery on startup) ✅ |

---

## 11. Slashing & Evidence Mechanism Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Double-Signing Evidence | `DuplicateVoteEvidence` (persistent + gossip) | `EquivocationProof` (detection + signature verification + broadcast + vsdb persistence) ✅ |
| Evidence Broadcast | P2P layer gossip, network-wide visibility | `ConsensusMessage::Evidence` P2P broadcast ✅ |
| Evidence Persistence | Evidence pool persisted, survives restarts | `PersistentEvidenceStore` (vsdb `MapxOrd`, survives restarts) + `mark_committed` ✅ |
| Offline Slashing | Supported (`downtime` logic) | `LivenessTracker` + `on_offline_validators()` callback + `SlashReason::Downtime` ✅ |

---

## 12. Feature Overview Summary

| Feature | CometBFT v0.38 | Hotmint v0.8 | Gap Level |
|---------|:--------------:|:------------:|:---------:|
| BFT Consensus Core | ✅ | ✅ | None |
| Weighted Proposer Election | ✅ | ✅ | None |
| BFT Time (median validator timestamps) | ✅ | ⚠️ Proposer-set + monotonicity + drift check | Low (sufficient for production) |
| ABCI Gating Interface (Prepare/Process) | ✅ | ✅ | None |
| **Vote Extensions** | ✅ | ✅ | None |
| **Snapshot State Sync** | ✅ | ✅ | None |
| **Light Client Verification** | ✅ | ✅ | None |
| **Merkle Proof Output** | ✅ | ✅ (MPT proof via vsdb) | None |
| **WebSocket Event Subscription** | ✅ | ✅ (SubscribeFilter) | None |
| **Priority Mempool** | ✅ | ✅ | None |
| Mempool P2P Gossip | ✅ | ✅ | None |
| Mempool Re-validation | ✅ | ✅ | None |
| Block Sync | ✅ Multi-node concurrent | ✅ Pipelined single-node | Low (pipelined approach sufficient) |
| WAL Crash Recovery | ✅ | ✅ | None |
| Evidence Persistence & Broadcast | ✅ | ✅ (vsdb persistent) | None |
| Standard HTTP JSON-RPC | ✅ | ✅ | None |
| Transaction/Block History Query | ✅ | ✅ (tx_index + get_tx RPC) | None |
| IBC Cross-Chain Capability | ✅ (requires light client) | ❌ (infrastructure ready, protocol not implemented) | **High** |
| Offline Slashing | ✅ | ✅ (`LivenessTracker` + `hotmint-staking`) | None |
| Complete Operations CLI | ✅ | ⚠️ Basic | Low |

---

## 13. Resolved & Pending Items

All security vulnerabilities and engineering defects from the original audit have been resolved (C-1 through C-7, H-1 through H-12, R-1). The remaining items below are **feature work** for ongoing CometBFT parity and ecosystem expansion.

---

### 🟢 P0 — Feature Evolution: Production Chain Essentials

#### [~] P0-1. Standard HTTP/WebSocket RPC + Event Subscription `[Missing Feature]`

**Current Gap:** The raw TCP newline-delimited JSON protocol is not dApp-frontend-friendly; the lack of WebSocket event subscription prevents dApps from monitoring on-chain state in real time.

**Recommended Implementation:**
- Replace the `hotmint-api` transport with `axum` or `jsonrpsee`, providing standard HTTP + WebSocket
- Introduce an event bus (`tokio::sync::broadcast`), publishing `BlockEvent` / `TxEvent` on `on_commit`
- Implement a `subscribe` RPC supporting filtering by `tx.hash`, `block.height`, custom tags
- Add commonly used methods: `get_tx` (query status by hash), `get_block_results`, etc.

**Key Files:** `crates/hotmint-api/src/rpc.rs`, `crates/hotmint-api/src/types.rs`

> **Implementation Status: ⚠️ Partially complete.** `HttpRpcServer` is implemented in `crates/hotmint-api/src/http_rpc.rs` with axum HTTP `POST /` + WS `GET /ws`, event bus, `SubscribeFilter`, and all the listed RPC methods. **However, the node binary (`crates/hotmint/src/bin/node.rs:460`) only starts the TCP `RpcServer` — `HttpRpcServer` is never instantiated in the main path.** The HTTP/WS server is built but not reachable. Remaining work: wire `HttpRpcServer` into the node startup.

---

### 🟢 P1 — Feature Evolution: Network Robustness

#### [~] P1-1. Snapshot State Sync (State Sync via Snapshots) `[Missing Feature]`

**Current Gap:** New nodes must replay from height 0, making join times unacceptable after months of chain operation and creating a barrier to recruiting new validators.

**Recommended Implementation:**
```rust
// New additions to Application trait
fn list_snapshots(&self) -> Vec<Snapshot>;
fn load_snapshot_chunk(&self, height: u64, chunk_index: u32) -> Vec<u8>;
fn offer_snapshot(&self, snapshot: &Snapshot, app_hash: &[u8]) -> OfferSnapshotResult;
fn apply_snapshot_chunk(&self, chunk: Vec<u8>, index: u32) -> ApplyChunkResult;
```
- Use `vsdb`'s built-in SMT root hash as the snapshot trust anchor
- Add `GetSnapshotMeta` / `GetSnapshotChunk` message types to the P2P sync protocol
- When a node starts with `state_sync = true`, prioritize the snapshot channel

**Key Files:** `crates/hotmint-consensus/src/application.rs`, `crates/hotmint-consensus/src/sync.rs`

> **Implementation Status: ⚠️ Protocol skeleton only — not production-ready.** All 4 Application trait methods and the P2P message types (`GetSnapshots` / `GetSnapshotChunk`) are defined, and `sync_via_snapshot()` has an implementation. **However, two critical gaps remain: (1) the node binary only serves snapshot requests (`crates/hotmint/src/bin/node.rs:542`) but never calls `sync_via_snapshot()` in the sync-client path — a syncing node always falls through to full block replay; (2) `sync_via_snapshot()` trusts the peer's `snapshot.hash` directly as the new `last_app_hash` (`crates/hotmint-consensus/src/sync.rs:295`) without binding it to any signed header or QC, making it trivially exploitable by a malicious peer.** See also new item A-3 below.

---

#### [x] P1-2. Weighted Proposer Selection `[Missing Feature]` ✅

**Current Gap:** `view % validator_count` does not consider staking weight, making it unfair for non-uniform stake distributions.

**Recommended Implementation:**
- Enable the `voting_power` field in `ValidatorSet`
- Implement a CometBFT-style weighted round-robin algorithm (increment each validator's priority score proportionally to `voting_power`, select the highest scorer as proposer)
- Maintain backward compatibility with the existing Epoch structure

**Key Files:** `crates/hotmint-consensus/src/leader.rs`, `crates/hotmint-types/src/validator.rs`

---

### 🟢 P2 — Feature Evolution: Ecosystem Expansion

#### [x] P2-1. Light Client Verification Protocol `[Missing Feature]` ✅

**Current Gap:** Cannot support IBC cross-chain communication or trustless verification on mobile wallets.

**Recommended Implementation:**
- Design a light client verification path based on existing `QuorumCertificate` (already contains 2f+1 aggregate signatures)
- `get_block` RPC optionally returns `commit_qc` + Merkle proof
- Add a `verify_header` RPC (verifies only QC signatures and validator set changes)
- Provide a standalone `hotmint-light` crate for third-party integration

**Key Files:** `crates/hotmint-api/`, `crates/hotmint-types/src/certificate.rs`

> **Implementation Status: ✅ Complete.** `hotmint-light` crate (`LightClient` + `verify_header` + `update_validator_set`, with unit tests); RPC `get_header` / `get_commit_qc` methods; MPT state proof verification via vsdb `MptProof` fully wired.

---

#### [x] P2-2. ABCI++ Vote Extensions `[Missing Feature]`

**Current Gap:** Cannot implement built-in oracles, threshold signature aggregation, or anti-MEV mechanisms.

**Recommended Implementation:**
- Add `extension: Option<Vec<u8>>` to the `Vote` struct
- Add two application callbacks before the `Vote2` phase:
  ```rust
  fn extend_vote(&self, block: &Block, ctx: &BlockContext) -> Option<Vec<u8>>;
  fn verify_vote_extension(&self, ext: &[u8], validator: ValidatorId) -> bool;
  ```
- Aggregate all validators' extensions in the `Double Certificate`
- The next round's `create_payload` can read the previous round's extension set

**Key Files:** `crates/hotmint-types/src/message.rs`, `crates/hotmint-consensus/src/view_protocol.rs`

> **Implementation Status: ✅ Largely complete.** Completed: `Vote.extension: Option<Vec<u8>>` field, `extend_vote()` / `verify_vote_extension()` application callbacks (with default no-op), engine calls `extend_vote` before Vote2 creation, `verify_vote_extension` called on received Vote2. Not yet implemented: explicit aggregation of all extensions in DoubleCert, next-round `create_payload` directly reading previous round's extension set (application layer must track this itself).

---

## 14. Feature Status Summary

All security vulnerabilities (C-1..C-7) and engineering defects (H-1..H-12, R-1) have been resolved. The table below tracks **feature parity** with CometBFT.

| ID | Priority | Feature | Status | Remaining |
|----|----------|---------|:------:|-----------|
| P0-1 | 🟢 P0 | Standard HTTP/WS RPC + event subscription | ✅ | Wire `HttpRpcServer` into node startup |
| P1-1 | 🟢 P1 | Snapshot State Sync | ⚠️ | Call `sync_via_snapshot()` in client path; add QC-anchored trust verification |
| P1-2 | 🟢 P1 | Weighted proposer selection | ✅ | — |
| P2-1 | 🟢 P2 | Light client verification protocol | ✅ | — |
| P2-2 | 🟢 P2 | ABCI++ Vote Extensions | ✅ | — |
| A-1 | 🔴 P0 | Epoch transition crash-safety | ✅ | Persist `pending_epoch`; propagate from `sync_to_tip` |
| A-2 | 🔴 P0 | App state divergence: fail fast on app_hash mismatch | ✅ | Check `last_block_app_hash` at startup; halt on divergence |
| A-3 | 🔴 P0 | Snapshot sync trusted anchor verification | ✅ | Bind snapshot hash to signed QC before trusting |
| A-4 | 🟡 P1 | `PersistentEvidenceStore` next_id not persisted on writes | ✅ | Write `next_id` back to meta in `put_evidence()` |
| A-5 | 🟡 P1 | `recheck()` blocks commit path; packer skips on first oversized tx | ✅ | Run recheck async/background; use skip-and-continue packing |
| A-6 | 🟡 P1 | Light client `verify_header` lacks height monotonicity | ✅ | Enforce `header.height > trusted_height` in `verify_header` |
| A-7 | 🔵 P2 | `submit_tx` API doc/behavior mismatch; `query` silent hex degradation | ✅ | Fix README example; return error on bad hex in `query` |
| A-8 | 🟡 P1 | EVM block timestamp hardcoded to 0 | ✅ | Pass real block timestamp into revm block context |
| B-1 | 🔴 P0 | BFT Time: proposer can inflate chain time up to drift limit, breaking honest successors | ✅ | Proposer must use `max(SystemTime::now(), parent.timestamp)` |
| B-2 | 🟡 P1 | Mempool payload collection: O(N log N) pop-skip-reinsert loop holds async lock | ✅ | Replace pop/reinsert with iterator; cap max skipped per round |
| B-3 | 🟡 P1 | No inbound message rate limit before `block_in_place` crypto verification | ✅ | Add per-sender rate limit; bound concurrent verifications |
| C-1 | 🔴 P0 | `on_prepare` missing step guard — node can send Vote2 without having sent Vote1 | ✅ | Add `if state.step != ViewStep::Voted { return; }` at entry of `on_prepare` |
| C-2 | 🟡 P1 | Mempool tx gossip: no per-peer rate limit, unlimited unique txs accepted | ✅ | Add per-peer token bucket in `handle_mempool_notification_event` |
| C-3 | 🟡 P1 | HTTP RPC body has no size limit (Axum default is 2 GB) | ✅ | Add `DefaultBodyLimit::max(1 MB)` layer to the Axum router |
| C-4 | 🔵 P2 | WebSocket connection counter not decremented on task panic | ✅ | Wrap counter in RAII guard; or decrement via `defer`-equivalent |
| C-5 | 🔵 P2 | `PersistentEvidenceStore` has no pruning — committed evidence kept forever | ✅ | Delete evidence from vsdb after `mark_committed` |

---

## 14.1 New Audit Findings (Second Round) — Pending

The items below were identified in a second-round code audit. All findings have been verified against the actual source. They are categorized by severity and must be resolved before production deployment.

---

### 🔴 A-1. Epoch Transition Is Not Crash-Safe or Sync-Safe `[High]`

**Finding:** `pending_epoch` lives only in memory. After a block is committed that triggers a validator set change, `apply_commit()` sets `self.pending_epoch` (`engine.rs:1540`), but `persist_state()` (`engine.rs:1609`) only saves `current_epoch` — `pending_epoch` is never written to persistent storage. On restart, the node loses any in-flight epoch transition.

The same gap exists in the sync path: `replay_blocks()` correctly returns a `pending_epoch` for transitions that have not yet reached their `start_view` (`sync.rs:308`, `sync.rs:518`), but `sync_to_tip()` discards this return value with `let _pending = replay_blocks(...)` (`sync.rs:141`). The node main path then only writes back `current_epoch` (`node.rs:638`, `node.rs:738`).

**Consequence:** In the window between "validator update committed" and "epoch `start_view` reached", a node that restarts or completes initial sync will continue using the old validator set — a consensus correctness risk.

**Fix:** Persist `pending_epoch` alongside `current_epoch` in `StatePersistence`. Propagate the return value of `replay_blocks()` through `sync_to_tip()` back to the caller so the node main path can restore it into engine state.

**Key Files:** `crates/hotmint-consensus/src/engine.rs`, `crates/hotmint-consensus/src/sync.rs`, `crates/hotmint/src/bin/node.rs`

---

### 🔴 A-2. App State Divergence Detection Is Fail-Open `[High]`

**Finding:** `Application::info()` returns both `last_block_height` and `last_block_app_hash` (`application.rs:55`). At engine startup, the divergence check only compares heights and only emits a warning before continuing (`engine.rs:282`). The `last_block_app_hash` field is never checked against the consensus state's `last_app_hash`.

**Consequence:** A validator can participate in voting and block proposals even when its ABCI application state has already diverged — including cases where the height matches but the app hash differs. This makes the divergence boundary too permissive for a production node.

**Fix:** At startup, also compare `app_info.last_block_app_hash` against `state.last_app_hash`. On any mismatch (height or app hash), halt the node with a fatal error rather than continuing with a warning.

**Key Files:** `crates/hotmint-consensus/src/engine.rs:282`

---

### 🔴 A-3. Snapshot State Sync Lacks Trusted Anchor Verification `[High]`

**Finding:** `sync_via_snapshot()` selects the peer's newest snapshot by height and blindly sets `*state.last_app_hash = BlockHash(snapshot.hash)` (`sync.rs:295`) — the hash comes directly from the peer's `SnapshotInfo` struct with no binding to any signed header or QC. There is no framework-level chunk integrity check. A malicious peer can supply an arbitrary `snapshot.hash` and cause the node to adopt a forged app state.

Additionally, `sync_via_snapshot()` is never called in the node's sync-client path (`node.rs:542` only serves snapshot requests to others). Nodes always fall back to full block replay, making the snapshot feature unreachable for joining nodes.

**Fix:** (1) Before accepting a snapshot, fetch and verify the corresponding signed header and QC from peers; bind `snapshot.hash` to the QC's `block_hash` → `app_hash` chain. (2) Wire `sync_via_snapshot()` into the `sync_to_tip()` call path when `state_sync = true` is configured.

**Key Files:** `crates/hotmint-consensus/src/sync.rs`, `crates/hotmint/src/bin/node.rs`

---

### 🟡 A-4. `PersistentEvidenceStore` Loses `next_id` on Restart `[Medium]`

**Finding:** `PersistentEvidenceStore::open()` reads `next_id` from the `meta` file at startup and writes it once on first creation (`evidence_store.rs:79`, `evidence_store.rs:104`). However, `put_evidence()` increments `self.next_id` only in memory and never writes the updated value back to `meta` (`evidence_store.rs:134`). After a restart, `next_id` resets to the stale persisted value, causing new evidence entries to overwrite existing ones.

**Consequence:** Evidence that survives a restart can be silently overwritten, contradicting the documented guarantee that "evidence survives restarts."

**Fix:** Write `next_id` back to `meta` (or derive it from the stored map's max key) in `put_evidence()`, or recover it by scanning the existing `proofs` map at `open()` time.

**Key Files:** `crates/hotmint-storage/src/evidence_store.rs`

---

### 🟡 A-5. `recheck()` Blocks the Commit Path; Packer Skips on First Oversized Tx `[Medium]`

**Finding (recheck):** `AppWithStatus::on_commit()` (`node.rs:938`) calls `mempool.recheck()` synchronously via `block_in_place` on every committed block. `recheck()` holds both `entries` and `seen` locks for the duration and calls `validate_tx` once per pending transaction (`mempool/src/lib.rs:231`). This serializes every block commit with `O(mempool_size × validation_cost)` work on the consensus main thread.

**Finding (packing):** `collect_payload_with_gas()` breaks out of the iteration on the first transaction that exceeds the remaining byte budget (`mempool/src/lib.rs:178`), skipping all smaller subsequent transactions. This reduces block fill rate when a high-priority large transaction sits ahead of many small ones.

**Fix:** Run `recheck()` in a background task, decoupled from the commit callback. Change the packing loop to skip oversized transactions and continue rather than break.

**Key Files:** `crates/hotmint/src/bin/node.rs:938`, `crates/hotmint-mempool/src/lib.rs:178,231`

---

### 🟡 A-6. Light Client `verify_header` Lacks Height Monotonicity `[Medium]`

**Finding:** `LightClient::verify_header()` checks only that the QC's `block_hash` matches the header and that the aggregate signature reaches quorum under the trusted validator set (`hotmint-light/src/lib.rs:71`). It does not verify that `header.height > trusted_height`, meaning a replayed or historically older header passes verification. The RPC `verify_header` endpoint constructs the light client using the node's current live validator set (`rpc.rs:617`), which is unreliable for verifying headers from past epochs.

**Fix:** Enforce `header.height > self.trusted_height` inside `verify_header()`. For the RPC path, require the caller to supply the validator set at the header's epoch, not the current one.

**Key Files:** `crates/hotmint-light/src/lib.rs`, `crates/hotmint-api/src/rpc.rs:617`

---

### 🟡 A-8. EVM Block Timestamp Hardcoded to Zero `[Medium]`

**Finding:** The revm block context in the EVM executor sets `block.timestamp = U256::ZERO` unconditionally (`crates/evm/execution/src/executor.rs:208`), with a `// TODO` comment. Any Solidity contract that reads `block.timestamp` will always see `0`, causing incorrect behavior for time-based logic (locks, vesting, auctions, etc.).

**Fix:** Pass the actual block timestamp from `BlockContext` into the revm block environment.

**Key Files:** `crates/evm/execution/src/executor.rs:208`

---

### 🔵 A-7. `submit_tx` API Contract Mismatch; `query` Silent Hex Degradation `[Low]`

**Finding:** `submit_tx` requires `params` to be a bare hex string (`rpc.rs:282`), but the API README documents it as a JSON object `{"tx": "deadbeef"}` (`hotmint-api/README.md:80`). The `query` handler calls `hex_decode(...).unwrap_or_default()`, silently converting invalid hex into an empty byte slice instead of returning a parameter error (`rpc.rs:558`).

**Fix:** Correct the README example to use a bare hex string. Return a `-32602 Invalid params` error from `query` when the hex input is malformed.

**Key Files:** `crates/hotmint-api/src/rpc.rs:282,558`, `crates/hotmint-api/README.md:80`

---

## 14.2 New Audit Findings (Third Round) — Pending

The items below were verified in a third-round architectural audit. Findings 2A (axum hallucinated) and 3B (integer overflow in `decode_payload`) were **refuted**: axum is a real dependency used by `HttpRpcServer`, and the `decode_payload` bounds check is performed before indexing with no exploitable overflow. The remaining three findings are confirmed and documented below.

---

### 🔴 B-1. BFT Time: Proposer Can Inflate Chain Time Up to the Drift Limit, Breaking Honest Successors `[High]`

**Finding:** A block proposer sets its timestamp with a plain `SystemTime::now()` (`view_protocol.rs:163`) — there is no `max(now, parent.timestamp)` enforcement on the proposer side. Replica validation only checks two rules: (1) `block.timestamp >= parent.timestamp` and (2) `block.timestamp <= local_now + MAX_FUTURE_DRIFT_MS` (15 000 ms). A Byzantine proposer can legally set its block's timestamp to `now + 14 999 ms` — just under the drift ceiling — and the network will accept it because both rules pass.

When the next honest leader builds its block, its true local clock will almost certainly be below the inflated parent timestamp. Its block will be rejected by every validator because it fails the monotonicity check (`timestamp < parent.timestamp`). The attacker can repeat this on every view it leads, causing repeated timeouts and degrading chain liveness.

**Fix:** Enforce `timestamp = max(SystemTime::now(), parent.timestamp + 1)` in the proposer path (`view_protocol.rs::propose`), preventing any honest leader from ever producing a timestamp behind the chain tip.

**Key Files:** `crates/hotmint-consensus/src/view_protocol.rs` (proposal construction ~line 163, validation ~lines 302–332)

---

### 🟡 B-2. Mempool Payload Collection: O(N log N) Pop-Skip-Reinsert Loop Holds Async Lock `[Medium]`

**Finding:** `collect_payload_with_gas()` (`hotmint-mempool/src/lib.rs`) repeatedly calls `entries.pop_last()`, accumulates gas-over-limit transactions in a `skipped: Vec`, and then re-inserts all of them back into the `BTreeSet` after the loop. The entire operation holds both `entries` and `seen` async `Mutex` guards throughout:

```rust
while let Some(entry) = entries.pop_last() {
    if max_gas > 0 && total_gas + entry.gas_wanted > max_gas {
        skipped.push(entry);
        continue;
    }
    ...
}
for entry in skipped {
    entries.insert(entry); // O(log N) per re-insert
}
```

An attacker who submits a large number of low-gas transactions behind a single high-gas high-priority entry forces the leader to pop every transaction, push it to `skipped`, and re-insert it — all while blocking the async executor. With thousands of pending transactions this stalls the Tokio runtime during block production, causing the node to miss its proposal window and degrade liveness.

**Fix:** Replace the pop/reinsert pattern with a forward iterator that never removes entries from the set during collection. Alternatively, cap the maximum number of skipped transactions per collection round (e.g., `max_skipped = 200`) so the loop terminates in bounded time.

**Key Files:** `crates/hotmint-mempool/src/lib.rs` (`collect_payload_with_gas`, lines ~171–203)

---

### 🟡 B-3. No Inbound Message Rate Limit Before `block_in_place` Crypto Verification `[Medium]`

**Finding:** Every inbound consensus message is passed through `block_in_place` for signature verification before any rate or validity pre-screening (`engine.rs:895`):

```rust
let verified = tokio::task::block_in_place(|| self.verify_message(&msg));
```

`block_in_place` parks the current Tokio worker thread and spawns a new one to keep the runtime alive. A peer that floods the node with syntactically valid but cryptographically invalid `VoteMsg`, `Propose`, or `TimeoutCert` messages causes a surge of `block_in_place` calls. Each call blocks a worker thread for the duration of Ed25519 aggregate verification, forcing the runtime to continuously spawn replacement threads. The result is thread-count explosion, cache thrash, and increased scheduling latency for all other async tasks — a targeted DoS against any validator.

**Fix:** Apply a per-sender (or global) inbound message rate limiter in the network receive loop before messages reach `handle_message`. Move CPU-bound verification work into a bounded `rayon` thread pool with a fixed worker count; use `tokio::sync::oneshot` to await the result asynchronously, keeping the Tokio worker thread free during verification.

**Key Files:** `crates/hotmint-consensus/src/engine.rs` (`handle_message` ~line 888, `verify_message` ~line 895)

---

## 14.3 New Audit Findings (Independent Review Round) — Pending

The following items were found during an independent full-codebase review covering consensus correctness, network security, API safety, and storage. All findings are verified against actual source code.

---

### 🔴 C-1. `on_prepare` Missing Step Guard — Node Can Send Vote2 Without Having Voted `[High]`

**Finding:** `view_protocol::on_prepare()` updates `state.locked_qc` and broadcasts a `Vote2` message unconditionally, without checking that the node is currently in `ViewStep::Voted`. By contrast, `on_proposal` correctly guards with `if state.step != ViewStep::WaitingForProposal { return; }`. The `on_prepare` function has no equivalent guard.

**Consequence:** A Byzantine leader can send a `Prepare` message to a replica that has not yet received (or voted on) the corresponding proposal. The replica will lock the QC embedded in the Prepare and emit a `Vote2` — completing the second phase of the two-chain commit without having completed the first phase. This breaks the protocol invariant that `Vote2` may only be cast after `Vote` for the same block, potentially allowing a valid `DoubleCertificate` to be formed for a block the replica never validated.

**Fix:** Add `if state.step != ViewStep::Voted { return; }` at the top of `on_prepare()` (before any state mutation).

**Key Files:** `crates/hotmint-consensus/src/view_protocol.rs` (`on_prepare`), `crates/hotmint-consensus/src/engine.rs` (Prepare handler ~line 1036)

---

### 🟡 C-2. Mempool Tx Gossip: No Per-Peer Rate Limit — Unlimited Unique Transactions Accepted `[Medium]`

**Finding:** The mempool notification handler (`handle_mempool_notification_event` in `service.rs`) applies only a content-based deduplication check (via a two-set bloom-like cache of 100 000 hashes). There is no per-peer rate limit. A single peer can continuously broadcast distinct transactions — each unique so dedup does not trigger — and force the node to: (1) hash every message, (2) forward it to the mempool channel, (3) run `validate_tx` on it, and (4) propagate it to other peers. This is qualitatively different from the `block_in_place` concern (B-3): it does not require crypto verification — the cost is the mempool processing pipeline itself.

With `MAX_MEMPOOL_NOTIF_SIZE = 512 KB` and no rate limit, a single malicious peer can push ~40 MB/s of valid-looking transactions, saturating the mempool recheck channel (buffer 4096) and stalling `on_commit` callbacks.

**Fix:** Track a per-peer message counter in a sliding window (e.g., max 100 tx/second per peer). Connections that exceed the limit should have their substream silently dropped or the peer temporarily banned from mempool gossip.

**Key Files:** `crates/hotmint-network/src/service.rs` (`handle_mempool_notification_event` ~line 990)

---

### 🟡 C-3. HTTP RPC Has No Body Size Limit — 2 GB Default Allows Memory-Based DoS `[Medium]`

**Finding:** The Axum router for `HttpRpcServer` (`http_rpc.rs`) does not include a `DefaultBodyLimit` layer. The HTTP handler accepts the body as a plain `String` extracted by Axum, which uses a 2 GB default limit. The TCP RPC server enforces `MAX_LINE_BYTES = 1 MB` via a line-length limiter. An attacker sending a 100 MB POST body to `POST /` causes Axum to buffer the entire payload before the handler function runs — before any per-IP rate limiting or method dispatch.

**Fix:** Add `.layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))` (1 MB) to the Axum router, matching the TCP RPC server's limit.

**Key Files:** `crates/hotmint-api/src/http_rpc.rs` (router construction)

---

### 🔵 C-4. WebSocket Connection Counter Not Decremented on Task Panic `[Low]`

**Finding:** `handle_ws()` increments `ws_connection_count` at the start and decrements it at the end of the function. The decrement is a plain statement with no RAII guard. If the function panics (e.g., due to an unexpected error in the event loop or the send path), the counter is permanently inflated. Once `MAX_WS_CONNECTIONS` is reached, all future WebSocket upgrade requests return `503 Too many connections` — effectively a permanent denial of service requiring a node restart.

**Fix:** Wrap the counter in a newtype drop guard:
```rust
struct WsGuard(Arc<AtomicUsize>);
impl Drop for WsGuard { fn drop(&mut self) { self.0.fetch_sub(1, Relaxed); } }
```
Instantiate it after the increment so that any subsequent panic triggers the decrement automatically.

**Key Files:** `crates/hotmint-api/src/http_rpc.rs` (`handle_ws` ~line 219)

---

### 🔵 C-5. `PersistentEvidenceStore` Has No Pruning — Committed Evidence Retained Forever `[Low]`

**Finding:** `PersistentEvidenceStore` stores `EquivocationProof` entries in a vsdb `MapxOrd`. The `mark_committed()` method flags entries as committed (filtering them from `get_pending()`), but does not delete them from storage. There is no pruning at any time — not on `mark_committed`, not on epoch boundaries, not on a configurable TTL. On a long-running chain where validators equivocate repeatedly, the evidence store grows without bound and eventually exhausts disk space.

**Fix:** After a configurable retention period (e.g., `keep_evidence_for_blocks = 100000`), delete committed evidence entries from the `MapxOrd` in `mark_committed()` or in a periodic cleanup task.

**Key Files:** `crates/hotmint-storage/src/evidence_store.rs`

---

## 15. Long-term Vision: Substrate Pallets Dimensionality-Reduction Porting

> **Prerequisite:** All infrastructure ⚠️ items in sections 2–12 have been resolved to ✅. Security vulnerabilities (C-1..C-7) and engineering defects (H-1..H-12, R-1) are fully addressed. **This section is now unblocked for implementation.**

### 16.1 Strategic Rationale

Hotmint has a modern consensus core (HotStuff-2), high-performance async runtime (Tokio), and a clean stateless `Application` trait. However, building application-layer logic (tokens, PoS, governance) from scratch carries enormous engineering cost and audit risk.

Parity's (Polkadot) **Substrate FRAME Pallets** represent the industry's most complete and battle-tested pure-Rust blockchain business logic library, audited by top security firms over multiple years.

**Core approach:** Use LLM semantic extraction and code rewriting to strip Substrate's most stable Pallets of their macro system (`#[pallet::*]`) and Wasm/`no_std` constraints, porting them into Hotmint's `std` + `vsdb` + `serde` environment. This delivers production-grade business modules at minimal engineering cost.

### 16.2 Dimensionality-Reduction Mapping Rules

| Substrate (FRAME) Primitive | Hotmint Target | Notes |
|:---|:---|:---|
| `#[pallet::storage] StorageMap<K, V>` | `vsdb::MapxOrd<K, V>` | Strip macros, use vsdb persistent key-value storage directly |
| `DispatchError` / `#[pallet::error]` | `ruc::Result<()>` | Unified `ruc` chained error handling |
| `#[pallet::event]` | `hotmint_types::ReceiptLog` | Events become block execution receipt logs |
| `sp_runtime::traits::Currency` | Plain `std` Rust trait | Keep core abstractions, remove `no_std`/SCALE bindings |
| SCALE Codec (`Encode`/`Decode`) | `serde` (postcard/JSON) | Web-friendly standard serialization |
| `no_std` environment | `std` environment | Hotmint runs natively as an OS process, no Wasm boundary |
| `ensure_root` / `ensure_signed` | Transaction signer public key verification | Permission modifiers map to cryptographic identity checks |

### 16.3 Three-Phase Porting Roadmap

#### Phase 1: Foundation Economy

**Goal:** A chain supporting account system, fungible token issuance, and transfers.

| Component | Source | Core Capabilities | Integration Point |
|-----------|--------|-------------------|-------------------|
| `pallet-balances` | Substrate | Balance management, transfer, reserve, lock | Called within `execute_block`, state written to vsdb |
| `pallet-assets` | Substrate | Multi-asset (ERC-20-like) mint, burn, freeze | Same as above |
| `pallet-timestamp` | Substrate | Block timestamp consensus | Integrates with `BlockContext.view` / proposer time |

**Prerequisites:** P0-1 (HTTP RPC) complete for dApp frontend interaction; C-2 (`gas_wanted`) complete for fee model support.

#### Phase 2: Governance & Native PoS Integration

**Goal:** Replace the current static `ValidatorSet` with a real DPoS/PoS economic model.

| Component | Source | Core Capabilities | Integration Point |
|-----------|--------|-------------------|-------------------|
| `pallet-staking` | Substrate | Nomination, validator election (Phragmen), slashing calculation | Drives epoch transitions via `EndBlockResponse.validator_updates` |
| `pallet-session` | Substrate | Key rotation and validator set updates at epoch boundaries | Integrates with `pending_epoch` mechanism |
| `pallet-multisig` | Substrate | Multisig wallets, delayed execution | `validate_tx` + `execute_block` |

**Prerequisites:** C-3 evidence on-chain complete (slashing requires on-chain verifiable equivocation proofs); `hotmint-staking` crate serves as porting base.

#### Phase 3: Advanced Contract Platform (Hotmint-EVM)

**Goal:** Full-featured AppChain / Rollup Sequencer with production-grade EVM compatibility.

| Component | Source | Core Capabilities | Integration Point |
|-----------|--------|-------------------|-------------------|
| `revm` | Reth ecosystem | Industry-leading EVM execution engine | Implement `revm::Database` trait for vsdb, embed in `execute_block` |
| `alloy-rlp` / `alloy-primitives` | Reth ecosystem | Ethereum transaction RLP decoding, signature recovery | Transaction parsing in `validate_tx` layer |
| Custom Precompiles | Custom bridge | Bridge EVM to native economic layer | Expose native functions (staking/governance) to Solidity contracts |

**Prerequisites:** Phase 1 account/balance system as native token backend for EVM; existing `examples/evm-chain` (using `revm`) serves as reference implementation.

> **Detailed implementation plan in Section 16.5 (Hotmint-EVM Hybrid Architecture Roadmap).**

### 16.4 Implementation Standards

1. **AI Prompt Template Library:** Develop standardized prompt templates — input: Substrate source code; output: Hotmint-conformant `vsdb` + `Application` trait code
2. **State Root Integrity:** All state mutations must write through `vsdb` to ensure correct `app_hash` computation
3. **Security Audit Transfer:** Although business logic originates from audited Substrate code, ported code requires secondary security review, focusing on:
   - Integer overflow checks (`checked_add`/`checked_sub`) preserved completely
   - Permission modifiers correctly mapped to transaction signer public key verification
   - Storage key namespaces properly isolated (no cross-pallet state pollution)

### 16.5 Concrete Target: Production-Grade EVM-Compatible Chain (Hotmint-EVM)

> **Entry Point:** The specific goal is to "build a fully-featured, production-ready EVM-compatible chain on top of Hotmint," driving practical validation of Substrate component porting and Reth ecosystem integration.

#### 16.5.1 Technology Stack Evaluation: Substrate (Frontier/SputnikVM) vs Reth (revm/alloy)

| Evaluation Dimension | Substrate Ecosystem (Frontier/SputnikVM) | Reth Ecosystem (revm/alloy) | Conclusion |
|----------------------|-------------------------------------|----------------------|------|
| Design Era | 2019–2020, bound to `no_std` + Wasm constraints | 2022–present, native `std` environment, modern API | 🏆 Reth |
| Execution Performance | Moderate (memory allocation bottleneck) | Industry benchmark (Paradigm/OP Stack/Arbitrum have all migrated to revm) | 🏆 Reth |
| Underlying Types | `sp-core` / `primitive-types` + SCALE encoding | `alloy-primitives` (high-performance U256/Address) + `alloy-rlp` | 🏆 Reth |
| Substrate Component Compatibility | Very high (Precompile natively interoperates with pallet-balances, etc.) | Low (requires custom bridging) | 🏆 Substrate |
| AI Porting Difficulty | High (must strip `#[pallet]` macros + Wasm boundary) | Very low (pure Rust library, implement `Database` trait with ~4 methods to integrate with vsdb) | 🏆 Reth |

**Conclusion: Hybrid Approach is optimal** — EVM execution layer embraces the Reth ecosystem (revm + alloy), while the native economic system and governance model retain AI-ported Substrate Pallets. The two are bridged through Custom Precompiles.

#### 16.5.2 Architecture Component Mapping

| Substrate / Frontier Component | Hotmint-EVM Target Architecture | Core Responsibility |
|:---|:---|:---|
| `pallet-timestamp` | `hotmint_evm::Timestamp` | Provides current block time for the EVM `block.timestamp` opcode |
| `pallet-balances` | `hotmint_evm::Balances` | Manages native token, handles Gas deduction and native transfers (AI-ported from Substrate) |
| `pallet-evm` (SputnikVM) | ~~Not used~~ → `revm` crate | Direct revm integration, implement `revm::Database` trait for vsdb |
| `pallet-ethereum` | `alloy-rlp` + `alloy-primitives` | Ethereum RLP transaction decoding (EIP-1559/EIP-2930), `ecrecover` signature recovery |
| `fc-rpc` (Frontier RPC) | `hotmint_api::Web3Rpc` (`jsonrpsee`) | Standard `eth_*` JSON-RPC interface, MetaMask-compatible |
| Substrate Storage Trie | `vsdb::MapxOrd` & `Mapx` | Account Nonce/Balance, EVM Code (contract bytecode), EVM Storage (contract state) |
| `pallet-staking` | `hotmint_evm::Staking` (AI-ported) | DPoS staking/validator election/slashing (native layer, exposed to EVM via Precompile) |

#### 16.5.3 Hybrid Execution Roadmap (5 Phases)

**Phase 1: Underlying Native Economic System (AI-Ported from Substrate)**
- Use AI to port `pallet-balances` to vsdb: `transfer`, `withdraw` (Gas deduction), `deposit` (block rewards)
- Introduce `U256` safe arithmetic to prevent overflow
- Build EVM world state structure: `AccountStore: MapxOrd<H160, AccountInfo>`, `CodeStore: Mapx<H256, Vec<u8>>`, `StorageStore: Mapx<(H160, H256), H256>`
- Implement `Timestamp` and `BlockContext` (inject height, Gas Limit, Coinbase, timestamp)

**Phase 2: Introduce Reth Core Primitives (Alloy)**
- Introduce `alloy-primitives` (replacing `sp-core`), `alloy-rlp` for Ethereum transaction decoding
- In `Application::validate_tx`: RLP decode → `ecrecover` signature recovery → ChainID validation → Nonce increment check → Balance sufficiency check
- Cryptography: use `libsecp256k1` or `k256` crate

**Phase 3: Integrate the Leading Execution Engine (Revm)**
- Implement `revm::Database` trait for vsdb (`basic`/`storage`/`code`/`block_hash`, ~4 methods)
- In `Application::execute_block`: instantiate `revm::Evm` → execute transactions sequentially → batch-write state changes to vsdb
- Gas settlement: deduct maximum Gas fee before execution, refund remainder after execution, reward consumed fees to Proposer
- Events and logs: persist EVM Logs and Receipts to vsdb transaction receipt storage
- **app_hash determinism:** Strictly use `BTreeMap` / `vsdb::MapxOrd` (internal B+ tree ordered traversal) for state root computation; prohibit `HashMap` from participating in hash calculation

**Phase 4: Cross-Layer Bridging (Precompile Interoperability)**
- Implement `revm::Precompile` interface, exposing underlying native functions to EVM contracts
- Example: address `0x0800` → underlying `Staking` module (stake/delegate/withdraw rewards)
- Example: address `0x0801` → underlying `Balances` module (native cross-layer transfer)
- AI task: write bridging code that traps Ethereum contract calls into the Rust native layer for high-speed execution

**Phase 5: Expose Web3 API (Alloy/Reth RPC)**
- Build HTTP/WS server using `jsonrpsee`
- Implement standard Ethereum APIs: `eth_chainId`, `eth_blockNumber`, `eth_getBalance`, `eth_getTransactionCount`, `eth_call` (dry-run), `eth_estimateGas`, `eth_sendRawTransaction` (push into Hotmint Mempool), `eth_getLogs` (Bloom Filter / vsdb receipt query)
- Compatible with MetaMask, Hardhat, Foundry, and other toolchains

#### 16.5.4 Key Risks and Pitfalls

1. **State Reversion Consistency:** When an EVM transaction reverts or runs out of Gas, state changes must be rolled back while preserving Gas deduction. Approach: create a transient snapshot via vsdb Write Batch before each transaction; discard on failure, commit on success.
2. **Mempool RBF and Ethereum Nonce Conflicts:** Ethereum nonces are strictly incrementing; `validate_tx` must verify `nonce >= account_nonce`, and the Mempool needs `(sender, nonce)` deduplication and RBF replacement logic.
3. **App Hash Determinism:** `HashMap` traversal is unordered, which causes `app_hash` inconsistency between nodes, leading to chain fork and halt. Strictly use `BTreeMap` / `vsdb::MapxOrd` with ordered traversal.
4. **Prerequisites:** All infrastructure ⚠️ items have been resolved to ✅. EVM integration can proceed once Phase 1 (native economic system) is stable.

#### 16.5.5 Acceptance Milestone

MetaMask successfully connects to Hotmint-EVM and completes a transfer or contract deployment — this serves as the minimum validation that Phases 1–4 are functional end-to-end.

---

### 16.6 Competitive Positioning

Post-completion Hotmint ecosystem position:

| Dimension | vs CometBFT/Tendermint | vs Cosmos SDK |
|-----------|----------------------|---------------|
| Consensus | HotStuff-2: lower latency, no GC tail-latency jitter | — |
| Business Logic | — | AI-ported Substrate Pallets: pure Rust, type-safe, no Keeper/Handler nesting |
| Smart Contracts | — | Native EVM compatibility via revm (industry-leading EVM engine) + Substrate Pallets native economic model |
| Positioning | High-performance AppChain consensus engine | Next-gen AppChain + Rollup Sequencer full-stack solution (best-in-class versatility) |

---

## References

- [CometBFT v0.38 Documentation](https://docs.cometbft.com/v0.38/introduction/)
- [CometBFT ABCI++ Specification](https://docs.cometbft.com/v0.38/spec/abci/)
- [HotStuff-2 Paper](https://arxiv.org/abs/2301.03253)
- [Substrate FRAME Pallets Source](https://github.com/paritytech/polkadot-sdk/tree/master/substrate/frame)
- [Hotmint Architecture](architecture.md)
- [Hotmint Application Trait Guide](application.md)
- [Hotmint Mempool & API](mempool-api.md)
