# Hotmint Security Audit & Evolution Roadmap

> **Report Version:** Based on Hotmint v0.8.6 / CometBFT v0.38
> **Generated:** 2026-03-24 | **Last Audit:** 2026-04-12 | **Last Document Sync:** 2026-04-12
> **Sources:** CometBFT feature gap analysis + four rounds of code security audit
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

Hotmint's combination of **Rust + HotStuff-2 + litep2p** gives it the potential to surpass CometBFT in core consensus algorithm and architectural modernization. All security vulnerabilities and engineering defects from all four audit rounds have been resolved (C-1..C-7, H-1..H-12, R-1, A-1..A-8, B-1..B-3, second-round C-1..C-5, third-round A3-1..A3-12, fourth-round A4-1..A4-8). All feature roadmap items are complete. Core feature parity with CometBFT has been achieved — the only remaining gap is:
- **Ecosystem Expansion Layer:** IBC cross-chain protocol (infrastructure ready, protocol not implemented)

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
| Pluggable Pool | Not pluggable (hardcoded CListMempool) | `MempoolAdapter` trait — chains supply their own pool impl (e.g. `EvmTxPool` with sender/nonce) ✅ |
| P2P Broadcast | Flood Mempool, peer Gossip | `NetworkSink::broadcast_tx()` — decoupled from pool impl, any chain can gossip ✅ |

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

## 13. Feature Evolution Items

All security vulnerabilities and engineering defects from all three audit rounds have been resolved. The items below document the completed feature roadmap.

---

### 🟢 P0 — Feature Evolution: Production Chain Essentials

#### [x] P0-1. Standard HTTP/WebSocket RPC + Event Subscription ✅

`HttpRpcServer` (`crates/hotmint-api/src/http_rpc.rs`) provides axum HTTP `POST /` + WS `GET /ws` with event bus, `SubscribeFilter`, and all standard RPC methods. The node binary starts it conditionally on `config.rpc.http_laddr` (`crates/hotmint/src/bin/node.rs:478-494`). The legacy TCP `RpcServer` remains for CLI tooling compatibility.

---

### 🟢 P1 — Feature Evolution: Network Robustness

#### [x] P1-1. Snapshot State Sync (State Sync via Snapshots) ✅

All 4 `Application` trait methods (`list_snapshots`, `load_snapshot_chunk`, `offer_snapshot`, `apply_snapshot_chunk`) are implemented. `sync_to_tip()` calls `sync_via_snapshot()` automatically when the catch-up gap exceeds `MAX_SYNC_BATCH` (`crates/hotmint-consensus/src/sync.rs:83-103`). The snapshot trust anchor is verified by fetching the signed anchor block and performing full QC aggregate-signature + quorum checks before adopting any snapshot state (`sync.rs:333-391`). The node binary serves snapshot chunks to peers and the sync client path is fully wired.

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

> **Implementation Status: ✅ Complete.** `Vote.extension: Option<Vec<u8>>` field, `extend_vote()` / `verify_vote_extension()` callbacks, engine calls `extend_vote` before Vote2, `verify_vote_extension` called on receipt. Extensions are aggregated in `VoteCollector.add_vote` when Vote2 quorum is reached (`vote_collector.rs:88-91`), stored in `DoubleCertificate.vote_extensions` (`engine.rs:1429`), and forwarded to the next round's `create_payload` via `BlockContext.vote_extensions` (`engine.rs:1682`, `view_protocol.rs:157`).

---

## 14. Feature Status Summary

All security vulnerabilities, engineering defects, and feature roadmap items are complete. No open items remain.

| ID | Priority | Feature | Status |
|----|----------|---------|:------:|
| P0-1 | 🟢 P0 | Standard HTTP/WS RPC + event subscription | ✅ |
| P1-1 | 🟢 P1 | Snapshot State Sync | ✅ |
| P1-2 | 🟢 P1 | Weighted proposer selection | ✅ |
| P2-1 | 🟢 P2 | Light client verification protocol | ✅ |
| P2-2 | 🟢 P2 | ABCI++ Vote Extensions | ✅ |

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

### 16.5 Concrete Target: Production-Grade EVM-Compatible Chain

> The EVM chain has been extracted into the **[nbnet](https://github.com/NBnet/nbnet)** repository.
> Full architecture documentation, component mapping, implementation roadmap, and production gap analysis have moved there:
>
> 📖 **[nbnet/docs/architecture.md](https://github.com/NBnet/nbnet/blob/master/docs/architecture.md)**

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

## 17. Known Users

Projects and chains built on Hotmint:

| Project | Description | Repository |
|:--------|:------------|:-----------|
| **nbnet** | EVM-compatible blockchain: revm execution, Ethereum JSON-RPC, EIP-1559 tx pool, custom precompiles | [github.com/NBnet/nbnet](https://github.com/NBnet/nbnet) |

---

## 18. Third-Round Full Codebase Audit (2026-04-07)

> **Audit Scope:** Full codebase (~16K LOC), all crates
> **Methodology:** Parallel subsystem audit per `.claude/docs/review-core.md`
> **Auditor:** Claude Code (automated deep analysis)
> **Findings:** 12 total (0 critical, 5 high, 5 medium, 2 low)

### 18.1 Summary

| Subsystem | Findings | Severity |
|-----------|:--------:|----------|
| Network | 2 | HIGH, MEDIUM |
| Storage | 3 | HIGH, HIGH, HIGH |
| API & ABCI | 3 | HIGH, MEDIUM, MEDIUM |
| Consensus | 1 | MEDIUM |
| Crypto | 1 | MEDIUM |
| Staking | 1 | LOW |
| Mgmt | 1 | LOW |
| Consensus core, Pacemaker, Sync, Mempool, Light client | 0 | Clean |

### 18.2 Findings

#### [x] A3-1. PeerMap Bidirectional Consistency Bug `[HIGH — Network]`

**Where:** `crates/hotmint-network/src/service.rs` — `PeerMap::insert`
**What:** `insert(vid, new_pid)` updates the forward map `validator_to_peer[vid] → new_pid` and inserts the new reverse mapping `peer_to_validator[new_pid] → vid`, but does NOT remove the stale reverse mapping `peer_to_validator[old_pid] → vid`. After a validator reconnects with a new PeerId, messages from the old PeerId are misattributed.
**Invariant:** INV-N2 (PeerMap bidirectional consistency). Cascades into `handle_epoch_change` — `remove(vid)` only cleans up the current reverse mapping, not any stale ones.
**Fix:**
```rust
pub fn insert(&mut self, vid: ValidatorId, pid: PeerId) {
    if let Some(old_pid) = self.validator_to_peer.insert(vid, pid) {
        self.peer_to_validator.remove(&old_pid);
    }
    self.peer_to_validator.insert(pid, vid);
}
```

---

#### [x] A3-2. Evidence Store Not Fsynced `[HIGH — Storage]`

**Where:** `crates/hotmint-storage/src/evidence_store.rs` — `put_evidence`, `persist_next_id`
**What:** `put_evidence()` writes to vsdb but `persist_next_id()` uses `std::fs::write()` without `sync_all()`. No `flush()` method exists on the `EvidenceStore` trait. `vsdb_flush()` is never called after evidence operations. A crash loses equivocation evidence — Byzantine validator escapes slashing.
**Invariant:** INV-ST4 (evidence persistence). Pattern 6.3 (evidence not crash-safe).
**Fix:** Add `sync_all()` to `persist_next_id()`. Add `flush()` to the `EvidenceStore` trait. Call `flush()` from the engine after `put_evidence()`.

---

#### [x] A3-3. Block Store put_block Not Atomic `[HIGH — Storage]`

**Where:** `crates/hotmint-storage/src/block_store.rs` — `put_block`
**What:** Two separate vsdb inserts (`by_height`, then `by_hash`) are not atomic. A crash after the first insert but before the second leaves an inconsistent state: `get_block_by_height()` returns a hash for a block that doesn't exist in `by_hash`.
**Invariant:** INV-ST2 (block store consistency). Pattern 6.2.
**Fix:** Reverse the order (insert `by_hash` first, then `by_height`) so that partial writes cause "not found by height" (recoverable) rather than "dangling hash reference" (inconsistent). Or wrap in a vsdb transaction if supported.

---

#### [x] A3-4. Evidence Not Flushed Before Consensus State Persist `[HIGH — Storage]`

**Where:** `crates/hotmint-consensus/src/engine.rs` — `process_commit_result`
**What:** `ev_store.mark_committed()` modifies vsdb maps but no flush follows. Then `persist_state()` flushes consensus state. A crash between evidence modification and consensus state flush loses evidence while consensus state survives.
**Invariant:** INV-ST4 combined with INV-ST1.
**Fix:** Add `ev_store.flush()` before `persist_state()`.

---

#### [x] A3-5. WebSocket Connection Limit TOCTOU Race `[HIGH — API]`

**Where:** `crates/hotmint-api/src/http_rpc.rs` — `ws_upgrade_handler`
**What:** The handler checks `ws_connection_count` then increments it later in `handle_ws` asynchronously. Multiple concurrent upgrade requests can all pass the check before any increment, exceeding `MAX_WS_CONNECTIONS = 1024`.
**Invariant:** INV-API2 (rate limiting). Check-then-act is non-atomic.
**Fix:** Use `compare_exchange` on the AtomicU64 to atomically check-and-increment, or acquire a semaphore permit before accepting the upgrade.

---

#### [x] A3-6. Missing Double Certificate View Ordering Validation `[MEDIUM — Consensus]`

**Where:** `crates/hotmint-consensus/src/engine.rs` — `validate_double_cert`
**What:** Validates same block hash and 2f+1 signatures for both QCs, but does not validate `outer_qc.view >= inner_qc.view`. A malformed DC with reversed view ordering could pass validation.
**Invariant:** INV-CS3 (DC validity requires QC2.view == QC1.view + 1). Missing defense-in-depth check.
**Fix:** Add `if dc.outer_qc.view < dc.inner_qc.view { return false; }`.

---

#### [x] A3-7. Ed25519 Signature Malleability — Non-Canonical S Accepted `[MEDIUM — Crypto]`

**Where:** `crates/hotmint-crypto/src/signer.rs` — `verify`
**What:** ed25519-dalek 2.2 default verification does not reject non-canonical signatures (where scalar S >= group order). Two different byte sequences can verify for the same (key, message). Current equivocation detection uses semantic content (not signature bytes) so practical impact is limited, but this violates INV-CR2.
**Invariant:** INV-CR2 (signature strictness / malleability protection). Pattern 7.3.
**Fix:** Enable `strict_signatures` feature: `ed25519-dalek = { ..., features = ["strict_signatures"] }`.

---

#### [x] A3-8. Relay Dedup Truncates Blake3 Hash to 8 Bytes `[MEDIUM — Network]`

**Where:** `crates/hotmint-network/src/service.rs` — relay deduplication
**What:** Relay message deduplication uses only the first 8 bytes of blake3 output (64-bit). With ~10K messages in the active set, birthday-bound collision probability becomes non-negligible, causing legitimate consensus messages to be silently dropped. Mempool dedup correctly uses full 32-byte hash.
**Invariant:** INV-N4 (correct dedup).
**Fix:** Use full 32-byte hash: `seen_active: HashSet<[u8; 32]>`.

---

#### [x] A3-9. Silent Hash Truncation/Padding in ABCI Protobuf Deserialization `[MEDIUM — ABCI]`

**Where:** `crates/hotmint-abci-proto/src/convert.rs` — `bytes_to_hash`
**What:** `bytes_to_hash` silently truncates or zero-pads hash fields that aren't exactly 32 bytes. A malformed ABCI message with a 16-byte hash would be padded with zeros and accepted, corrupting cryptographic integrity.
**Invariant:** INV-API3 (frame integrity). Cryptographic fields must be strictly validated.
**Fix:** Return `Err` instead of silently padding.

---

#### [x] A3-10. Application Error Messages Exposed in RPC Responses `[MEDIUM — API]`

**Where:** `crates/hotmint-api/src/rpc.rs`
**What:** Internal application errors are forwarded verbatim to untrusted clients via `format!("query failed: {e}")`. An attacker can craft inputs to extract internal state details.
**Invariant:** Information leakage to untrusted clients.
**Fix:** Return generic error messages to clients; log detailed errors server-side only.

---

#### [x] A3-11. ValidatorId Not Derived From Public Key `[LOW — Staking]`

**Where:** `crates/hotmint/src/bin/node.rs` — ValidatorId lookup
**What:** ValidatorId is assigned manually via `--validator_id` flag and looked up in genesis, rather than derived deterministically from the public key (e.g., `hash(pubkey)`). Different genesis documents could assign different IDs to the same key.
**Invariant:** INV-CR5 (key derivation determinism).
**Fix:** Derive ValidatorId from public key hash at registration time.

---

#### [x] A3-12. Missing SAFETY Comments on Unsafe Blocks `[LOW — Mgmt]`

**Where:** `crates/hotmint-mgmt/src/local.rs` — two `libc::kill` blocks
**What:** Both `unsafe` blocks lack required `// SAFETY:` documentation. The code is sound (PIDs are triple-validated: `read_pid` + `is_running` + `is_cluster_node`), but the project convention requires explicit safety justification.
**Invariant:** Project convention (no undocumented unsafe).
**Fix:** Add `// SAFETY:` comments explaining the invariants.

---

### 18.3 Clean Areas

| Subsystem | What Was Verified |
|-----------|-------------------|
| **Consensus core** | INV-CS1 (voting safety), INV-CS2 (2f+1 quorum), INV-CS4 (commit completeness), INV-CS5 (view monotonicity), INV-CS7 (epoch/view validation); vote dedup; equivocation detection |
| **Pacemaker** | INV-CS6 all three guarantees (fires per view, exponential backoff 1.5x capped 30s, reset on view change) |
| **Sync** | Cursor advances after each batch; terminates when caught up; no infinite loop; no interference with active consensus |
| **Mempool** | INV-MP1 (ordering consistency), INV-MP2 (no duplicates), INV-MP3 (eviction correctness), INV-MP4 (RBF atomicity); lock ordering correct |
| **Light client** | INV-CR3 (batch verification all-or-nothing); height monotonicity; hash chain verification |
| **Domain separation** | INV-CR1 — all 5 message types (Vote, Proposal, Prepare, Wish, Status) include chain_id, epoch, view, type tag |
| **Epoch transitions** | Atomic; +2 view delay correct; slashing verified before penalty; unbonding prevents slash evasion |
| **ABCI framing** | INV-API3 — 64MB bound; length validated before allocation |
| **API read-only** | INV-API1 — no write locks in RPC handlers |
| **Concurrency** | No parking_lot guards across .await; all channels bounded on consensus path; select! branches cancel-safe |

---

## 19. Fourth-Round Full Codebase Audit (2026-04-12)

> **Audit Scope:** Full codebase (~16K LOC), all crates
> **Base Commit:** `4044a24` (v0.8.6)
> **Methodology:** Parallel subsystem audit per `.claude/docs/review-core.md`
> **Auditor:** Claude Code (automated deep analysis)
> **Findings:** 8 total (0 critical, 1 high, 0 medium, 7 low)

### 19.1 Summary

| Subsystem | Findings | Severity |
|-----------|:--------:|----------|
| Sync | 1 | HIGH |
| Engine | 1 | LOW |
| Network | 4 | LOW, LOW, LOW, LOW |
| ABCI Proto | 1 | LOW |
| Facade & Mgmt | 1 | LOW (style) |
| Consensus core, Pacemaker, Types, Crypto, Storage, Mempool, Light client, Staking, API | 0 | Clean |

### 19.2 Findings

#### [x] A4-1. `replay_blocks` Drops Pending Epoch From Prior Batch — Cross-Epoch Sync Broken `[HIGH — Sync]`

**Where:** `crates/hotmint-consensus/src/sync.rs:428`
**What:** `replay_blocks()` initializes its local `pending_epoch` to `None` instead of reading from `state.pending_epoch`. When an epoch transition is triggered in batch N (e.g., block at view 99 sets `start_view=101`) but the activation view falls in batch N+1, the pending epoch stored by `sync_to_tip` at line 178 into `state.pending_epoch` is never loaded by the next `replay_blocks` call. Batch N+1's blocks in the new epoch are then verified against the OLD validator set, causing spurious QC verification failures and sync abort.
**Invariant:** Sync convergence invariant. Pattern 2.3 (sync loop). The infrastructure to fix this exists (`SyncState.pending_epoch` field, A-1 comments) but the load side was never wired up.
**Fix:**
```rust
// sync.rs line 428 — change:
let mut pending_epoch: Option<Epoch> = None;
// to:
let mut pending_epoch: Option<Epoch> = state.pending_epoch.take();
```

---

#### [x] A4-2. Equivocation Evidence Not Flushed Immediately After Detection `[LOW — Engine]`

**Where:** `crates/hotmint-consensus/src/engine.rs:1340-1354`
**What:** `handle_equivocation()` calls `evidence_store.put_evidence()` but never calls `evidence_store.flush()`. The next `flush()` only occurs inside `process_commit_result` (line 1566). If the node crashes between detecting equivocation and the next block commit, the evidence is lost from durable storage. Note: A3-2 fixed the missing `flush()` method on the trait and A3-4 fixed the commit-path flush ordering; this is the *detection-path* gap that remained.
**Invariant:** INV-ST4 (evidence persistence). Pattern 6.3.
**Fix:** Add `self.evidence_store.as_ref().map(|s| s.flush());` after `put_evidence` in `handle_equivocation()`.

---

#### [x] A4-3. PeerMap.insert Does Not Clean Stale Reverse Mapping When PeerId Is Reused `[LOW — Network]`

**Where:** `crates/hotmint-network/src/service.rs:67-72`
**What:** When `insert(vid, pid)` is called and `pid` already maps to a different ValidatorId in `peer_to_validator`, the old ValidatorId's forward entry in `validator_to_peer` is left dangling. A `send_to(old_vid)` would route to `pid`, which now belongs to a different validator. Note: A3-1 fixed the old_pid forward cleanup; this is the symmetric reverse-direction case.
**Invariant:** INV-N2 (PeerMap bidirectional consistency). Requires PeerId collision or misconfiguration to trigger — very low practical risk.
**Fix:**
```rust
if let Some(old_vid) = self.peer_to_validator.insert(pid, vid) {
    if old_vid != vid {
        self.validator_to_peer.remove(&old_vid);
    }
}
```

---

#### [x] A4-4. Eviction Does Not Clean Mempool Peer Tracking `[LOW — Network]`

**Where:** `crates/hotmint-network/src/service.rs:882-886`
**What:** When a non-validator peer is evicted (C-1 eviction to make room for a validator), it is removed from `connected_peers` and `notif_connected_peers` but NOT from `mempool_notif_connected_peers` or `mempool_peer_rate`. Evicted peers remain in the mempool broadcast set and the rate-limit HashMap leaks entries.
**Invariant:** Peer tracking consistency. Not a consensus correctness bug — sends to evicted peers fail silently at litep2p layer.
**Fix:** Add to eviction block: `self.mempool_notif_connected_peers.remove(&evict_peer); self.mempool_peer_rate.remove(&evict_peer);`

---

#### [x] A4-5. ConnectionClosed Does Not Clean Mempool Peer Tracking `[LOW — Network]`

**Where:** `crates/hotmint-network/src/service.rs:902-913`
**What:** Same issue as A4-4 but for normal TCP disconnects. Consensus notification peers are eagerly cleaned on `ConnectionClosed`, but mempool notification peers and rate-limit entries are not. The code explicitly handles the "TCP drops before `NotificationStreamClosed`" race for consensus (lines 906-912) but not for mempool.
**Invariant:** Peer tracking consistency (asymmetry with consensus notification cleanup).
**Fix:** Add to ConnectionClosed handler: `self.mempool_notif_connected_peers.remove(&peer); self.mempool_peer_rate.remove(&peer);`

---

#### [x] A4-6. Relay Broadcasts to `connected_peers` Instead of `notif_connected_peers` `[LOW — Network]`

**Where:** `crates/hotmint-network/src/service.rs:519`
**What:** Message relay iterates `self.connected_peers` (all TCP-connected peers) rather than `self.notif_connected_peers` (peers with an open notification substream). The handle_command `Broadcast` path (line 941) correctly uses `notif_connected_peers`. `send_sync_notification` to peers without an open substream fails silently (`let _ =`), generating unnecessary failed attempts per relayed message.
**Invariant:** Consistency between relay and broadcast paths. Not a correctness bug.
**Fix:** Change `for &other in &self.connected_peers` to `for &other in &self.notif_connected_peers`.

---

#### [x] A4-7. `assert!` Panic in `bytes_to_hash` on Malformed ABCI Protobuf Input `[LOW — ABCI Proto]`

**Where:** `crates/hotmint-abci-proto/src/convert.rs:324-328`
**What:** `bytes_to_hash()` calls `assert!(bytes.len() == 32)` which panics on non-32-byte non-empty input. Reachable from any protobuf decode path (`Block`, `EquivocationProof`, `EndBlockResponse`) when the remote ABCI application sends a malformed hash field. Note: A3-9 fixed silent truncation/padding; the remaining case is non-empty input of wrong length, which now panics instead of returning an error.
**Invariant:** Defense-in-depth. The ABCI socket is local-only (Unix domain socket), so exploitation requires a co-located buggy application. The consensus engine should never panic on wire input.
**Fix:** Replace `assert!` with a fallible conversion returning `Result<BlockHash, DecodeError>` and propagate the error.

---

#### [x] A4-8. Inline-Path Rule Violations Across 3 Files `[LOW — Style]`

**Where:**
- `crates/hotmint-mgmt/src/lib.rs` — `std::process::{Command, Stdio}` used 11 times without import
- `crates/hotmint/src/bin/node.rs` — `hotmint::api::types::ValidatorInfoResponse` used 5 times without import
- `crates/hotmint/src/config.rs` — `litep2p::crypto::ed25519::*` used 7 times without import

**What:** Multiple files use fully-qualified paths 5-11 times without a top-level `use` import.
**Invariant:** Project convention — 3+ inline uses of the same path should be imported.
**Fix:** Add appropriate `use` imports at the top of each file.

---

### 19.3 Clean Areas

| Subsystem | What Was Verified |
|-----------|-------------------|
| **Consensus core** | INV-CS1 (voting safety via justify QC rank), INV-CS2 (2f+1 quorum with `ceil(2n/3)` weighted), INV-CS3 (DC validity: same block hash + view ordering + dual QC verification), INV-CS4 (commit walks full ancestor chain), INV-CS5 (view monotonicity via `advance_view_to` guard), INV-CS7 (epoch/view filtering before dispatch); duplicate vote prevention at collector + aggregate level; equivocation detection |
| **Pacemaker** | INV-CS6 all three guarantees (timeout fires per view via dedicated select! branch, exponential backoff `1.5^n` capped at `max_timeout`, reset on DC/TC); no lock-across-await |
| **Types & crypto** | INV-CR1 (domain separation complete: all 5 message types with chain_id, epoch, view, type tag), INV-CR2 (`verify_strict` enabled), INV-CR3 (batch verify all-or-nothing via `ed25519_dalek::verify_batch`), INV-CR4 (no Blake3 truncation for identity), INV-CR5 (ValidatorId deterministic within genesis) |
| **Storage** | INV-ST1 (crash atomicity via WAL fsync), INV-ST2 (block store consistency: by_hash before by_height), INV-ST3 (WAL fsynced before app commit), INV-ST5 (recovery reads persisted state correctly); no parking_lot across await; correct read/write lock separation |
| **Mempool** | INV-MP1 (BTreeSet/HashMap always updated in sync), INV-MP2 (no duplicates: `seen` checked before insert), INV-MP3 (eviction removes lowest-priority from both structures), INV-MP4 (RBF atomic: both locks held throughout); lock ordering `entries` before `seen`; pool size accurate |
| **API** | INV-API1 (zero `.write()` calls in API crate), INV-API2 (per-IP rate limiting at 100 req/s + 100K IP cap), INV-API4 (WS count via AtomicUsize RAII guard); TCP connection limit 256 via Semaphore; 1MB line-length limit; 30s read timeout; WS backpressure via `Lagged` drop |
| **ABCI** | INV-API3 (64MB frame cap on all 4 read/write paths); no sensitive data in error messages |
| **Light client** | INV-CR3 (batch verify soundness); quorum + aggregate signature verification + height monotonicity; forged headers cannot pass without 2f+1 valid signatures |
| **Staking** | Delegation uses `checked_add` / `saturating_sub`; evidence cryptographically verified before slash; unbonding logic correct; epoch transitions applied at +2 view delay |
| **Engine** | Message dispatch validates epoch/view before processing; signature verification before state mutation; rate limiting at 100 msg/s per sender; `SharedBlockStore` lock never held across `.await`; all select! branches cancel-safe; bounded channels (8192 consensus, 4096 commands) |
| **Unsafe blocks** | Both `libc::kill` in mgmt: SAFETY comments present and accurate; PID validated via `is_running` + `is_cluster_node`; TOCTOU inherent to POSIX PID management, mitigated as well as possible |
| **Concurrency (global)** | No parking_lot guards across `.await` in any crate; all channels on consensus path bounded; tokio::select! branches cancel-safe; consistent lock ordering across all subsystems |

---

## References

- [CometBFT v0.38 Documentation](https://docs.cometbft.com/v0.38/introduction/)
- [CometBFT ABCI++ Specification](https://docs.cometbft.com/v0.38/spec/abci/)
- [HotStuff-2 Paper](https://arxiv.org/abs/2301.03253)
- [Substrate FRAME Pallets Source](https://github.com/paritytech/polkadot-sdk/tree/master/substrate/frame)
- [Hotmint Architecture](architecture.md)
- [Hotmint Application Trait Guide](application.md)
- [Hotmint Mempool & API](mempool-api.md)
