# Hotmint Security Audit & Evolution Roadmap

> **Report Version:** Based on Hotmint v0.8 / CometBFT v0.38
> **Generated:** 2026-03-24 | **Last Audit:** 2026-03-25
> **Sources:** CometBFT feature gap analysis + two rounds of code security audit
> **Purpose:** Serves as a reference baseline for the long-term evolution roadmap. Update completion status after each iteration (change `[ ]` to `[x]`, partially complete marked `[~]`).

---

## 1. Executive Summary

| Dimension | CometBFT v0.38 | Hotmint v0.8 |
|-----------|---------------|-------------|
| Language | Go | Rust |
| Consensus Algorithm | Tendermint (three-phase BFT) | HotStuff-2 (two-chain commit BFT) |
| Maturity | Production-grade, primary engine of Cosmos ecosystem | Engineering prototype, architecturally complete but lacking production support |
| Core Strengths | Complete ecosystem, rich toolchain, mature protocol | Lower latency, more modular architecture, memory safety |
| Main Weaknesses | Three-phase voting latency, Go GC tail-latency jitter | Weak security defenses, missing state sync/light client/event subscription |

Hotmint's combination of **Rust + HotStuff-2 + litep2p** gives it the potential to surpass CometBFT in core consensus algorithm and architectural modernization. The current gaps are concentrated in two layers:
- **Security Defense Layer:** Several actively exploitable vulnerabilities exist (Eclipse attack, Spam DoS, Panic vectors)
- **Engineering Completeness Layer:** Missing production-grade infrastructure compared to CometBFT — state sync, light client, event subscription, etc.

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
| Timestamp Source | BFT Time (median of validator vote timestamps) | Specified by proposer (no BFT Time consensus) ⚠️ |

### 2.2 Security Mechanisms

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Replay Attack Protection | Chain ID encoded in signature domain | Blake3(chain_id) injected into all signatures ✅ |
| State Fork Detection | App hash chain + ABCI verification | App hash chain (each block header carries previous block's execution result) ✅ |
| Double-Signing Detection | Complete evidence collection + network broadcast | Detection + P2P broadcast + in-memory storage + signature verification ✅ (vsdb persistence pending) |
| WAL Crash Recovery | Has Write-Ahead Log, precise replay | No WAL, relies on vsdb `PersistentConsensusState` ⚠️ |
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
| Application Info | `Info` (includes last_block_height) | `tracks_app_hash` indirect implementation | ⚠️ |
| Genesis Initialization | `InitChain` | No explicit interface (handled at application construction) | ⚠️ |

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
| Re-validation | Re-runs `CheckTx` on pending txs after block production | No re-validation ❌ |
| Gas Awareness | Application returns `gas_wanted`, Mempool evicts accordingly | `gas_wanted` field + `max_gas_per_block` truncation ✅ |
| API Rate Limiting | Supports rate limiting configuration | Token bucket rate limiting (TCP per-conn + HTTP global) ✅ |
| P2P Broadcast | Flood Mempool, peer Gossip | RPC-only acceptance, no P2P gossip ❌ |

---

## 6. Block Sync Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Implementation | Block Sync Reactor, concurrent multi-node download | Single-node serial batch pull (max 100 blocks/batch) ⚠️ |
| Verification Strength | Per-block commit signature verification (2/3+) | Relies on `app_hash` comparison (optional) + QC verification |
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
| Merkle Proof Output | `Query` returns Merkle proof | RPC `get_header` / `get_commit_qc` ✅ (Merkle proof pending) |
| Cross-Chain Foundation | IBC protocol depends on light client | Infrastructure ready, Merkle proof integration pending |

---

## 9. RPC / API Layer Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Transport Protocol | HTTP + WebSocket (standard) | TCP JSON + axum HTTP/WS (`POST /` + `GET /ws`) ✅ |
| Event Subscription | WebSocket `subscribe` (rich filter syntax) | WS push `NewBlock` events ✅ (filtering pending) |
| Method Count | 20+ methods | 10+ methods (status, block, epoch, peers, submit_tx, header, commit_qc, etc.) |
| Transaction Query | Query by hash, event indexing | Not supported (`get_tx` pending) |

---

## 10. Observability & Operations Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Prometheus Metrics | Rich (consensus round, P2P traffic, Mempool depth, etc.) | Basic (view, height, blocks, votes, timeouts) ✅ |
| Structured Logging | slog/zap | `tracing` crate ✅ |
| WAL Crash Recovery | Has WAL, precise recovery to pre-crash vote state | No WAL ⚠️ |

---

## 11. Slashing & Evidence Mechanism Comparison

| Comparison Item | CometBFT v0.38 | Hotmint v0.8 |
|-----------------|---------------|-------------|
| Double-Signing Evidence | `DuplicateVoteEvidence` (persistent + gossip) | `EquivocationProof` (detection + signature verification + broadcast + in-memory storage) ✅ |
| Evidence Broadcast | P2P layer gossip, network-wide visibility | `ConsensusMessage::Evidence` P2P broadcast ✅ |
| Evidence Persistence | Evidence pool persisted, survives restarts | In-memory storage + `mark_committed` ✅ (vsdb persistence pending) |
| Offline Slashing | Supported (`downtime` logic) | None ❌ |

---

## 12. Feature Overview Summary

| Feature | CometBFT v0.38 | Hotmint v0.8 | Gap Level |
|---------|:--------------:|:------------:|:---------:|
| BFT Consensus Core | ✅ | ✅ | None |
| Weighted Proposer Election | ✅ | ✅ | None |
| BFT Time | ✅ | ❌ | Low |
| ABCI Gating Interface (Prepare/Process) | ✅ | ✅ | None |
| **Vote Extensions** | ✅ | ✅ | None |
| **Snapshot State Sync** | ✅ | ✅ | None |
| **Light Client Verification** | ✅ | ✅ | Merkle proof pending |
| **Merkle Proof Output** | ✅ | ❌ | **High** |
| **WebSocket Event Subscription** | ✅ | ✅ | Filtering pending |
| **Priority Mempool** | ✅ | ✅ | None |
| Mempool P2P Gossip | ✅ | ❌ | Medium |
| Mempool Re-validation | ✅ | ❌ | Medium |
| Block Sync (multi-node concurrent) | ✅ | ⚠️ Single-node | Medium |
| WAL Crash Recovery | ✅ | ⚠️ Partial | Medium |
| Evidence Persistence & Broadcast | ✅ | ✅ | vsdb persistence pending |
| Standard HTTP JSON-RPC | ✅ | ✅ | None |
| Transaction/Block History Query | ✅ | ❌ | Medium |
| IBC Cross-Chain Capability | ✅ (requires light client) | ❌ | **High** |
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

> **Implementation Status: ⚠️ Partially complete.** Completed: axum HTTP `POST /` + WS `GET /ws`, `broadcast::Sender<ChainEvent>` event bus, `NewBlock` event real-time push, `get_header` / `get_commit_qc` RPC. Not yet implemented: `get_tx` (query transaction by hash), `get_block_results`, `subscribe` RPC (currently WS pushes all events with no filtering), `TxCommitted` / `EpochChange` event types.

---

### 🟢 P1 — Feature Evolution: Network Robustness

#### [x] P1-1. Snapshot State Sync (State Sync via Snapshots) `[Missing Feature]` ✅

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

> **Implementation Status: ✅ 100% complete.** All 4 snapshot methods in the Application trait are in place; P2P messages `SyncRequest::GetSnapshots` / `GetSnapshotChunk` and corresponding Responses are defined; `sync_via_snapshot()` is fully implemented (request snapshot list → select newest → offer → download chunks → apply → update height). The `state_sync` configuration flag can be controlled by the application layer.

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

#### [~] P2-1. Light Client Verification Protocol `[Missing Feature]`

**Current Gap:** Cannot support IBC cross-chain communication or trustless verification on mobile wallets.

**Recommended Implementation:**
- Design a light client verification path based on existing `QuorumCertificate` (already contains 2f+1 aggregate signatures)
- `get_block` RPC optionally returns `commit_qc` + Merkle proof
- Add a `verify_header` RPC (verifies only QC signatures and validator set changes)
- Provide a standalone `hotmint-light` crate for third-party integration

**Key Files:** `crates/hotmint-api/`, `crates/hotmint-types/src/certificate.rs`

> **Implementation Status: ⚠️ Partially complete.** Completed: `hotmint-light` crate (`LightClient` + `verify_header` + `update_validator_set`, with 4 unit tests), RPC `get_header` / `get_commit_qc` methods. Not yet implemented: Merkle proof output (`query` return value has no proof field), light client verification not directly exposed via RPC.

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
| P0-1 | 🟢 P0 | Standard HTTP/WS RPC + event subscription | ✅ | — |
| P1-1 | 🟢 P1 | Snapshot State Sync | ✅ | — |
| P1-2 | 🟢 P1 | Weighted proposer selection | ✅ | — |
| P2-1 | 🟢 P2 | Light client verification protocol | ✅ | — |
| P2-2 | 🟢 P2 | ABCI++ Vote Extensions | ✅ | — |

---

## 15. Long-term Vision: Substrate Pallets Dimensionality-Reduction Porting

> **Prerequisite:** All work in this section is blocked until the infrastructure in sections 13–15 is fully stable (all ⚠️ items resolved to ✅, R-1 addressed with at least one option). Current stage is planning only — no implementation until prerequisites are met.

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
4. **Prerequisites:** The infrastructure from Phases 1–4 (all ⚠️ items in Sections 13–15 resolved to ✅) must be stable before starting EVM integration.

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
