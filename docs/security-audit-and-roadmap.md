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

## 13. Complete Fix List

The following items are ordered by **real risk priority**, combining findings from the CometBFT feature gap analysis and the code security audit. Each item is tagged: `[Security Vulnerability]` / `[Engineering Defect]` / `[Missing Feature]`.

---

### 🔴 Critical — Security Vulnerabilities (Blocking Production Launch)

#### [x] C-1. Eclipse Attack: P2P Validator Connection Slots Lack Protection `[Security Vulnerability]`

**Location:** `crates/hotmint-network/src/service.rs`

**Problem:** The network layer limits total connections by `max_peers`. Once the limit is reached, all new inbound connections are rejected. An attacker can use a large number of Sybil Nodes to fill all connection slots, preventing legitimate validators from establishing P2P links and causing the consensus network to lose liveness.

**Fix:**
- During the inbound handshake phase, check whether the peer is in the current `ValidatorSet`
- If the peer is a validator node, forcibly evict a low-reputation non-validator connection to make room, even if `max_peers` has been reached
- Maintain dedicated "Reserved Slots" for validator nodes, with a count no less than `validator_count`

**Severity:** 🔴 High — can cause network-level Liveness Failure

> **Implementation Status: ✅ Largely complete.** Inbound handshake now checks `peer_to_validator`; validators are not rejected even when exceeding `max_peers`. Not yet implemented: dedicated reserved slot counter, proactive eviction of non-validator connections to make room for validators.

---

#### [~] C-2. FIFO Mempool DoS: Spam Transactions Block Legitimate Transactions `[Security Vulnerability × Missing Feature]`

**Location:** `crates/hotmint-mempool/src/lib.rs`, `crates/hotmint-api/src/rpc.rs`

**Problem A (Spam DoS):** The Mempool is a priority-less FIFO queue (default limit 10,000 entries) with no rate limiting on the API layer. An attacker can instantly submit 10,000 tiny but `validate_tx`-passing junk transactions to the RPC endpoint, filling the queue. All subsequent legitimate transactions are rejected, effectively achieving a DoS attack on the chain's transaction channel.

**Problem B (DeFi Unusable):** Without Gas/Priority ordering, fee-bidding mechanisms cannot work, making DeFi applications non-functional.

**Fix:**
- **Mempool Refactor:** Replace `VecDeque` with `BinaryHeap` (sorted by `priority`); add `priority: u64` and `gas_wanted: u64` to the `validate_tx` return value; evict lowest-priority transactions when the pool is full
- **Source Quotas:** Limit the concurrent occupancy ratio per IP/PeerId (e.g., max 10% of total capacity)
- **API Rate Limiting:** Add per-IP rate limiting to `submit_tx` in the `hotmint-api` RPC layer (e.g., token bucket algorithm)
- **`collect_payload` Extension:** Add `max_gas_per_block` with gas-cumulative truncation

**Key Files:** `crates/hotmint-mempool/src/lib.rs`, `crates/hotmint-consensus/src/application.rs`

**Severity:** 🔴 High — can achieve on-chain transaction channel DoS

> **Implementation Status: ⚠️ Partially complete.** Completed: BTreeSet priority queue + RBF, `TxValidationResult { valid, priority }` return value, full-pool eviction of lowest priority, token bucket rate limiting (100 tx/sec per connection). Not yet implemented: `gas_wanted` field, per-IP/PeerId source quotas (current rate limiting is per-connection only — attacker can bypass with multiple connections), `collect_payload`'s `max_gas_per_block` truncation.

---

#### [~] C-3. Missing Evidence Broadcast: Double-Signers Can Escape Punishment `[Security Vulnerability × Missing Feature]`

**Location:** `crates/hotmint-consensus/src/vote_collector.rs`, `crates/hotmint-consensus/src/engine.rs` (~line 991)

**Problem:** `vote_collector.rs` correctly detects double-signing and generates `EquivocationProof`, and the engine subsequently calls `app.on_evidence(proof)` to pass it to the application layer. However, Hotmint has no mechanism to broadcast evidence to the entire network — if the node that detects the misbehavior is not the current Leader, the evidence remains only in the local application process. A malicious actor can double-sign against a subset of non-proposing nodes, and these proofs can never make it on-chain, completely evading slashing penalties. Additionally, evidence is not persisted — it is lost on node restart.

**Fix:**
- Add a `/hotmint/evidence/1` P2P broadcast protocol in `hotmint-network`
- When the engine detects an `EquivocationProof`, **immediately** broadcast it to the entire network via P2P
- When the Leader constructs the next Block, **mandatorily** embed collected uncommitted Evidence in the Block Header or Payload
- Add an `EvidenceStore` (vsdb persistence) in `hotmint-storage` so evidence survives restarts

**Key Files:** `crates/hotmint-consensus/src/engine.rs`, `crates/hotmint-storage/`, `crates/hotmint-network/src/service.rs`

**Severity:** 🔴 High — slashing mechanism is effectively inoperative; malicious validators can double-sign at no cost

> **Implementation Status: ⚠️ Partially complete.** Completed: `ConsensusMessage::Evidence` message type, `broadcast_evidence()` via existing notification protocol (not a dedicated protocol), `EvidenceStore` trait (put/get_pending/mark_committed/all), `MemoryEvidenceStore` in-memory implementation, engine immediately broadcasts + stores on detecting double-signing, gossip evidence stored and application layer notified upon receipt. Not yet implemented: vsdb persistent storage (currently in-memory only — lost on restart), Leader packing evidence into Block (code comment: "full block inclusion is a later step"), `mark_committed` is never called.

#### [x] C-4. Proposal ancestor constraint missing `[Safety Violation]` ✅

**Location:** `crates/hotmint-consensus/src/view_protocol.rs` (`on_proposal`, ~line 200)

**Problem:** `on_proposal` never verifies that `block.parent_hash == justify.block_hash`, nor that the parent block exists in the store. A Byzantine leader can propose a block forking from an arbitrary point in the chain — honest nodes will accept and store it, potentially voting for a block that does not extend the certified chain.

**Fix:**
- Before accepting a proposal, verify `block.parent_hash == justify.block_hash`
- Verify parent block exists in store (or is genesis) before voting

**Severity:** 🔴 Critical — violates chain extension safety property

---

#### [x] C-5. Vote2Msg path missing vote_type check — phase confusion `[Safety Violation]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (~line 975, `ConsensusMessage::Vote2Msg`)

**Problem:** The `Vote2Msg` handler does not verify `vote.vote_type == VoteType::Vote2`. The `VoteMsg` handler correctly checks `vote.vote_type == VoteType::Vote` and rejects mismatches, but `Vote2Msg` accepts any vote_type. A malicious peer can send a `Vote2Msg` containing a Vote1-phase vote, bypassing Vote1 path constraints and potentially forming a DoubleCert from votes in the wrong phase.

**Fix:** Add `if vote.vote_type != VoteType::Vote2 { return Ok(()); }` at the top of the Vote2Msg handler, mirroring the VoteMsg guard.

**Severity:** 🔴 Critical — phase confusion can produce invalid DoubleCert

---

#### [x] C-6. Evidence gossip accepted without cryptographic verification `[Safety Violation]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (~line 1098, `ConsensusMessage::Evidence`)

**Problem:** When the engine receives an `Evidence(proof)` message via gossip, it calls `app.on_evidence()` and stores the proof without verifying the two conflicting signatures. A malicious peer can forge an `EquivocationProof` with arbitrary signatures, triggering application-layer slashing logic against an innocent validator.

**Fix:**
- Before accepting evidence, verify both `signature_a` and `signature_b` using the alleged validator's public key and the corresponding signing_bytes
- Reject and drop the proof if either signature is invalid

**Severity:** 🔴 Critical — forged evidence can slash innocent validators

---

#### [x] C-7. `apply_commit` + `persist_state` not atomic — crash recovery gap `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (`apply_commit` ~line 1296, `persist_state` ~line 1475)

**Problem:** `apply_commit` executes blocks (mutating application state) and flushes to the block store, but consensus state (`last_committed_height`, `current_view`, `locked_qc`) is only persisted later in `persist_state()` during `advance_view_to`. If the node crashes between `apply_commit` completing and `persist_state` being called:
- Application state reflects committed blocks
- Block store reflects committed blocks
- But on-disk consensus state still shows the previous height/view
- On restart the node may re-execute already-committed blocks, causing state divergence

**Fix:** Call `persist_state()` at the end of `apply_commit` (after `s.flush()`), or adopt a write-ahead log (WAL) that records the commit intent before executing.

**Severity:** 🔴 Critical — crash window causes irrecoverable state divergence

---

### 🟡 High — Engineering Safety (Should Fix Before Production Deployment)

#### [x] H-1. O(N) Signature Verification CPU DoS Risk `[Security Vulnerability]` ✅

**Location:** `crates/hotmint-crypto/src/aggregate.rs`

**Problem:** The current "aggregate signature" is actually N Ed25519 signatures concatenated, then verified by iterating `ed25519_dalek::Verifier::verify` with O(N) time complexity. Full traversal verification is required every time a QC, DC, or block sync is received. An attacker can frequently send seemingly valid QCs with random data, forcing the node to consume massive CPU, causing view-change timeouts (Liveness failure).

**Fix (choose one):**
1. **Option A (long-term):** Introduce true aggregate signatures (e.g., BLS12-381), compressing N signature verifications into a single pairing operation with O(1) verification cost
2. **Option B (short-term):** Move `verify_aggregate` to a dedicated CPU thread pool via `tokio::task::spawn_blocking` to avoid blocking the consensus engine's main event loop; also add source authentication for QC/DC messages from unknown sources (only accept messages from known validator PeerIds)

**Key Files:** `crates/hotmint-crypto/src/aggregate.rs`

**Severity:** 🟡 Medium — can trigger Liveness failure in large validator sets (100+ nodes)

> **Implementation Status: ✅ 100% complete.** Option B fully implemented: `verify_aggregate` now uses `ed25519_dalek::verify_batch` (Bos-Coster batch verification); `verify_message` / `validate_double_cert` / Wish QC verification all wrapped in `tokio::task::block_in_place`; signature domain binds epoch to prevent replay.

---

#### [x] H-2. `pending_epoch` Force-Unwrap Panic Vector `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/engine.rs` (Epoch transition logic)

**Problem:** The epoch transition code contains `self.pending_epoch.take().unwrap()` as a force unwrap. If consensus state fails to correctly inject `pending_epoch` after a crash recovery or abnormal edge case (e.g., the application layer returned ValidatorUpdates but the process crashed and restarted immediately), this `unwrap()` will trigger an unrecoverable Panic when the consensus engine reaches a specific view height, causing the node to permanently crash at that height.

**Fix:**
- Replace `unwrap()` with `ok_or`/`expect` combined with `Result` propagation
- If `pending_epoch` is missing, fall back to a safe state (continue running with the current Epoch) or re-request state sync from the application layer
- Add unit tests for this path (simulating epoch transition after crash recovery)

**Key Files:** `crates/hotmint-consensus/src/engine.rs`

**Severity:** 🟡 Medium — can trigger unrecoverable node crash on specific crash recovery paths

---

#### [x] H-3. zstd Compression-Side `unwrap()` Panic Vector `[Engineering Defect]` ✅

**Location:** `crates/hotmint-network/src/codec.rs`

**Problem:** The decompression side correctly sets a `MAX_DECOMPRESSED_SIZE` limit, but the compression side uses `zstd::encode_all(..., ZSTD_LEVEL).unwrap()`. When the OS runs out of memory or certain extremely large payloads trigger an internal zstd error, this `unwrap()` will crash the underlying network service, taking the node offline.

**Fix:**
- Propagate the `Result` returned by `zstd::encode_all` upward
- On compression failure, drop the message or disconnect the corresponding client connection — **never** let a Panic propagate to the main process

**Key Files:** `crates/hotmint-network/src/codec.rs`

**Severity:** 🟡 Medium — can trigger node shutdown under unusual network loads

---

#### [x] H-4. ABCI IPC Communication Lacks Timeout Protection `[Engineering Defect]` ✅

**Location:** `crates/hotmint-abci/src/client.rs`

**Problem:** During synchronous frame read/write, if the application process (Go/other language implementation) hangs but the Unix socket remains open, there are currently no explicit `ReadTimeout` / `WriteTimeout` settings. This causes the Rust consensus engine to hang indefinitely in `tokio::task::spawn_blocking`, blocking the entire consensus process from producing blocks and effectively halting the chain.

**Fix:**
- Set strict read/write timeouts on the underlying `UnixStream` (recommended to align with `base_timeout_ms`, or use a fixed 5s timeout)
- After timeout, report the IPC failure as a fatal error, triggering application-layer reconnection or node restart flow

**Key Files:** `crates/hotmint-abci/src/client.rs`

**Severity:** 🟡 Medium — can cause the consensus engine to hang permanently when the application process malfunctions

---

#### [x] H-5. Vote Signatures Missing `epoch_number`, Cross-Epoch Replay Risk `[Security Concern]` ✅

**Location:** `crates/hotmint-types/src/vote.rs` (`signing_bytes` method)

**Problem:** The current `signing_bytes` contains `chain_id_hash + view + block_hash`. `view` is globally monotonically increasing, which is safe in the short term. However, if cross-epoch state resets, major validator set changes, or chain fork repairs are implemented in the future, signatures legitimately generated in an old Epoch could be used to construct forged votes in a new Epoch (cross-epoch replay attack).

**Fix:**
- Explicitly add an `epoch_number` field to `signing_bytes`
- When verifying votes, also verify that `epoch_number` matches the current Epoch
- This change affects on-wire data format and requires versioned migration

**Key Files:** `crates/hotmint-types/src/vote.rs`

**Severity:** 🟡 Low-Medium — cannot be triggered under the current model, but affects security of future extensions

#### [x] H-6. P2P handshake empty — no chain/genesis/version isolation `[Engineering Defect]` ✅

**Location:** `crates/hotmint-network/src/service.rs` (~line 205, 410)

**Problem:** The litep2p notification protocol is initialized with `.with_handshake(vec![])` (empty) and `.with_auto_accept_inbound(true)`. Inbound substreams are unconditionally accepted via `ValidationResult::Accept`. Any peer can connect and inject consensus messages regardless of chain_id, genesis hash, or protocol version. This allows cross-chain message injection in multi-chain deployments.

**Fix:** Include `chain_id_hash + protocol_version` in the handshake bytes; in `ValidateSubstream`, verify the handshake matches before accepting.

**Severity:** 🟡 High — cross-chain message injection in multi-network environments

---

#### [x] H-7. Sync replay epoch transition applies immediately, runtime delays to start_view `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/sync.rs` (~line 413) vs `crates/hotmint-consensus/src/engine.rs` (~line 1431)

**Problem:** During `replay_blocks`, epoch transitions take effect immediately (`*state.current_epoch = Epoch::new(...)`) after the committing block. During normal consensus, the engine stores a `pending_epoch` and only applies it when `new_view >= e.start_view`. This creates a semantic mismatch: sync-replaying nodes use the new validator set immediately, while live consensus nodes use it only after `start_view`. Blocks in the gap window may be verified against different validator sets.

**Fix:** `replay_blocks` should defer the epoch transition to `start_view`, or replay blocks in the gap window using the old validator set and switch at the correct view.

**Severity:** 🟡 High — validator set mismatch during/after sync can cause verification failures

---

#### [x] H-8. `SharedStoreAdapter` panics on lock contention (`try_read/try_write`) `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/store.rs` (lines 48–97)

**Problem:** Every `BlockStore` method in `SharedStoreAdapter` uses `self.0.try_read().expect("store read lock contended")` or `try_write().expect(...)`. If the `tokio::sync::RwLock` is held by another task at the time of the call, `try_*` returns `Err` and `.expect()` panics, crashing the node. This is used by the sync responder path which runs concurrently with the consensus engine.

**Fix:** Replace `try_read().expect()` with `.read().await` (or `blocking_read()` in sync contexts), or accept `Result` and propagate errors gracefully.

**Severity:** 🟡 High — concurrent access causes node crash

---

#### [x] H-9. Node binary defaults `evidence_store: None` — evidence system inert `[Engineering Defect]` ✅

**Location:** `crates/hotmint/src/bin/node.rs` (~line 724)

**Problem:** The production node binary constructs `EngineConfig` with `evidence_store: None`. Despite all the evidence infrastructure (EvidenceStore trait, MemoryEvidenceStore, broadcast_evidence, gossip handling), the store is never wired in. `handle_equivocation` silently skips storage; gossip evidence silently skips storage.

**Fix:** Initialize `evidence_store: Some(Box::new(MemoryEvidenceStore::new()))` in the node binary. For persistence, implement a vsdb-backed store.

**Severity:** 🟡 High — evidence system is dead code in production

---

#### [x] H-10. HTTP RPC rate limiter created per-request — effectively disabled `[Engineering Defect]` ✅

**Location:** `crates/hotmint-api/src/http_rpc.rs` (~line 95)

**Problem:** `json_rpc_handler` creates a fresh `TxRateLimiter::new(TX_RATE_LIMIT_PER_SEC)` on every HTTP request. Each request gets a full token allowance, making the rate limit meaningless. An attacker can submit unlimited `submit_tx` calls by sending unlimited HTTP requests.

**Fix:** Store a shared `TxRateLimiter` (or per-IP map) in `HttpRpcState` and pass it to each request handler. The TCP RPC server correctly creates one limiter per connection.

**Severity:** 🟡 High — mempool spam via HTTP endpoint

---

#### [x] H-11. ABCI IPC `ValidateTx` returns only `bool`, client hardcodes `priority: 0` `[Engineering Defect]` ✅

**Location:** `crates/hotmint-abci-proto/proto/abci.proto` (ValidateTxResponse), `crates/hotmint-abci/src/client.rs` (~line 172)

**Problem:** The IPC wire protocol (`ValidateTxResponse { bool ok }`) does not carry a `priority` field. The Rust ABCI client maps `ok=true` to `TxValidationResult { valid: true, priority: 0 }`. This means out-of-process applications (Go, etc.) cannot signal transaction priority, rendering the priority mempool queue, eviction, and RBF features inoperative for ABCI apps.

**Fix:** Extend `ValidateTxResponse` with `uint64 priority` (and optionally `uint64 gas_wanted`). Update client to read and forward to `TxValidationResult`.

**Severity:** 🟡 Medium — priority mempool disabled for all ABCI applications

---

#### [x] H-12. Sync replay does not persist `commit_qc` `[Engineering Defect]` ✅

**Location:** `crates/hotmint-consensus/src/sync.rs` (`replay_blocks`, ~line 375)

**Problem:** `replay_blocks` stores blocks via `state.store.put_block()` but never calls `put_commit_qc()` for synced blocks, despite the QC being available in the input tuple `(Block, Option<QuorumCertificate>)`. After sync, the node cannot serve commit QCs to other syncing peers or to the light client RPC (`get_commit_qc`), creating a "sync hole" that degrades network resilience.

**Fix:** After `put_block`, call `state.store.put_commit_qc(block.height, qc.clone())` when `qc.is_some()`.

**Severity:** 🟡 Medium — synced nodes cannot serve commit proofs to peers or light clients

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
- Use `vsdb`'s built-in MPT root hash as the snapshot trust anchor
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

## 14. Full Priority Summary Table

| ID | Severity | Description | Status | Missing Items |
|----|----------|-------------|:------:|---------------|
| C-1 | 🔴 High | Eclipse attack: validator connection slots unprotected | ✅ | — |
| C-2 | 🔴 High | FIFO Mempool DoS + no API rate limiting | ✅ | — |
| C-3 | 🔴 High | Missing evidence broadcast, double-signers escape punishment | ✅ | — |
| H-1 | 🟡 Medium | O(N) signature verification CPU DoS risk | ✅ | — |
| H-2 | 🟡 Medium | `pending_epoch.unwrap()` panic vector | ✅ | — |
| H-3 | 🟡 Medium | zstd compression-side `unwrap()` panic vector | ✅ | — |
| H-4 | 🟡 Medium | ABCI IPC no read/write timeout, can hang permanently | ✅ | — |
| H-5 | 🟡 Low-Med | Signature missing `epoch_number`, cross-epoch replay risk | ✅ | — |
| **C-4** | 🔴 Critical | Proposal missing parent_hash == justify.block_hash check | ✅ | — |
| **C-5** | 🔴 Critical | Vote2Msg no vote_type == Vote2 guard — phase confusion | ✅ | — |
| **C-6** | 🔴 Critical | Evidence gossip accepted without signature verification | ✅ | — |
| **C-7** | 🔴 Critical | apply_commit + persist_state not atomic — crash gap | ✅ | — |
| **H-6** | 🟡 High | Empty P2P handshake — no chain/version isolation | ✅ | — |
| **H-7** | 🟡 High | Sync epoch transition immediate vs runtime delayed | ✅ | — |
| **H-8** | 🟡 High | SharedStoreAdapter try_read/try_write panics on contention | ✅ | — |
| **H-9** | 🟡 High | Node binary defaults evidence_store: None | ✅ | — |
| **H-10** | 🟡 High | HTTP rate limiter per-request — effectively disabled | ✅ | — |
| **H-11** | 🟡 Medium | ABCI IPC ValidateTx returns bool, priority hardcoded 0 | ✅ | — |
| **H-12** | 🟡 Medium | Sync replay doesn't persist commit_qc | ✅ | — |
| P0-1 | 🟢 P0 | Standard HTTP/WS RPC + event subscription system | ✅ | — |
| P1-1 | 🟢 P1 | Snapshot State Sync | ✅ | — |
| P1-2 | 🟢 P1 | Weighted proposer selection | ✅ | — |
| P2-1 | 🟢 P2 | Light client verification protocol | ⚠️ | Merkle proof output (requires vsdb MPT integration) |
| P2-2 | 🟢 P2 | ABCI++ Vote Extensions | ✅ | — |
| R-1 | 🟢 Low | RwLock fair lock RPC congestion | ✅ | Migrated to parking_lot::RwLock |

---

## 15. Medium-term Improvements (Second Audit Round)

Items from the second code audit. Low severity but relevant to long-term throughput.

#### [ ] R-1. `tokio::sync::RwLock` Fair Lock Causes RPC Congestion Under High Concurrency `[Performance]`

**Location:** `crates/hotmint-api/src/rpc.rs`, `crates/hotmint-consensus/src/engine.rs`

**Problem:** The consensus engine and the RPC layer share `Arc<tokio::sync::RwLock<Box<dyn BlockStore>>>`. `tokio::sync::RwLock` is a fair lock — when a writer (`store.write().await`) is queued, new readers are also blocked. If the RPC endpoint is public-facing and hit by bursty `get_block` / `get_commit_qc` traffic, accumulated read locks can force the consensus engine's `put_block` writes to queue, slightly slowing block confirmation.

**Current mitigation:** All lock holds are synchronous HashMap lookups with no `.await` points (microsecond-level). The `try_propose` write lock is already scoped to release before any `.await`. Actual contention probability is very low.

**Suggested optimization paths (medium-term):**
- **Option A:** Provide RPC with a lock-free read-only snapshot handle (leveraging VSDB snapshot capabilities), fully decoupling RPC reads from consensus writes
- **Option B:** Publish latest block header/height via `Arc<tokio::sync::watch::Sender>`, making basic status queries zero-contention
- **Option C:** Migrate to `parking_lot::RwLock` (guards are `Send`, usable across `.await`), or `dashmap` / lock-free concurrent structures

**Severity:** Low — only impacts TPS under extreme RPC concurrency (thousands of QPS)

---

## 16. Long-term Vision: Substrate Pallets Dimensionality-Reduction Porting

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
