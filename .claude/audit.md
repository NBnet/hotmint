# Audit Findings

> Auto-managed by /x-review and /x-fix.
> Historical audit rounds (formerly in `docs/security-audit-and-roadmap.md`) consolidated here.
> Earlier rounds (C-1..C-7, H-1..H-12, R-1, A-1..A-8, B-1..B-3, C2-1..C2-5) were resolved prior to Round 3 and are not itemized.

## Open

*(No open findings)*

---

## Resolved — Audit Backlog Fix

> **Scope:** Full audit backlog from latest `/x-review`
> **Findings:** 44 total (7 critical, 21 high, 12 medium, 4 low)
> **Status:** Fixed all 44; no new Won't Fix entries.

### [CRITICAL] consensus: previous-epoch QCs and DCs use the wrong validator set
- **Where**: crates/hotmint-consensus/src/engine.rs:726-747,803-823,1185-1217,1255-1274,1645-1691; crates/hotmint-consensus/src/sync.rs:475-503
- **What**: QC/DC validation passes `self.state.validator_set` to aggregate verification even when `qc.epoch` is a previous epoch. Several paths also skip quorum checks when `qc.epoch != current_epoch`.
- **Why**: Violates 2f+1 quorum and cross-epoch verification invariants (technical-patterns.md 3.1, 3.3, 4.2). A previous-epoch certificate can be rejected for liveness or accepted under the wrong power/key set for safety.
- **Suggested fix**: Add a `validator_set_for_epoch(qc.epoch)` helper that returns only current or retained previous sets, always verify signatures and quorum against that set, and reject unknown epochs.

### [CRITICAL] consensus: previous-epoch votes can mint current-epoch QCs
- **Where**: crates/hotmint-types/src/vote.rs:17-50; crates/hotmint-consensus/src/engine.rs:656-777,1061-1079,1139-1168; crates/hotmint-consensus/src/vote_collector.rs:42-99
- **What**: `Vote` does not carry an epoch. `verify_message()` accepts current or previous epoch signatures, then vote handlers pass `self.state.current_epoch.number` to `VoteCollector`, which stamps any formed QC with the current epoch.
- **Why**: Violates vote epoch binding and replay protections (technical-patterns.md 3.3, 5.1, 7.1). A replayed previous-epoch vote for the same view/block can be counted into a current-epoch QC.
- **Suggested fix**: Include epoch in `Vote`, or have verification return the matched epoch and collect votes in epoch-scoped buckets that stamp QCs with the verified epoch.

### [CRITICAL] consensus/sync: non-contiguous block heights can be certified and committed
- **Where**: crates/hotmint-consensus/src/view_protocol.rs:323-382; crates/hotmint-consensus/src/commit.rs:77-109,115-192; crates/hotmint-consensus/src/sync.rs:440-463,513-620
- **What**: Proposal, sync replay, and commit walking check parent hashes but never enforce `block.height == parent.height + 1`.
- **Why**: Violates ancestor-chain commit completeness (technical-patterns.md 1.4). A certified block at height 100 can point to genesis and advance `last_committed_height` while heights 1-99 are missing.
- **Suggested fix**: Require parent availability and exact height continuity before voting, during sync replay, and while walking commit ancestors.

### [CRITICAL] consensus: pending epoch transitions can be overwritten before activation
- **Where**: crates/hotmint-consensus/src/commit.rs:171-187; crates/hotmint-consensus/src/engine.rs:1056-1057,1621-1623,1742-1787; crates/hotmint-consensus/src/sync.rs:605-616
- **What**: The engine stores only one `pending_epoch`. A later committed block with validator updates before the earlier transition reaches `start_view` replaces the previous pending transition.
- **Why**: Violates deterministic validator-set transition rules (technical-patterns.md 4.1, 4.3). Nodes can lose or apply different update sequences around epoch boundaries.
- **Suggested fix**: Reject or defer validator updates while a pending epoch exists, or chain pending transitions deterministically in a queue keyed by start view.

### [CRITICAL] crypto: block hash is ambiguous for variable-length evidence signatures
- **Where**: crates/hotmint-types/src/block.rs:100-114; crates/hotmint-types/src/crypto.rs:6-10
- **What**: `Block::compute_hash()` concatenates `Signature(Vec<u8>)` fields and payload bytes without length prefixes.
- **Why**: Violates canonical hash identity/domain binding (technical-patterns.md 7.1). Different malformed evidence signature/payload splits can produce the same byte stream and block hash.
- **Suggested fix**: Make cryptographic fields fixed-size (`[u8; 64]` signatures, `[u8; 32]` keys) or length-prefix every variable-length field in the block hash format.

### [CRITICAL] crypto/staking: validator identity is not bound to public keys or vote signatures
- **Where**: crates/hotmint-types/src/validator.rs:64-76,168-193; crates/hotmint-types/src/vote.rs:31-50; crates/hotmint-staking/src/manager.rs:45-72
- **What**: `ValidatorSet::new()` and staking registration accept caller-supplied validator IDs and duplicate or malformed public keys, while vote signing bytes omit `validator_id`.
- **Why**: Violates deterministic validator identity and duplicate-vote accounting invariants (technical-patterns.md 3.2, 7.1). The same signature can validate for two validator IDs that share a key and be counted as distinct power.
- **Suggested fix**: Validate Ed25519 keys, reject duplicate public keys, derive or verify `ValidatorId` from the public key, and include `validator_id` in vote signing bytes in a protocol bump.

### [CRITICAL] staking: equal-power validator ties are nondeterministic
- **Where**: crates/hotmint-staking/src/manager.rs:366-375; crates/hotmint-staking/src/store.rs:67-68
- **What**: `formal_validator_list()` sorts only by voting power. Equal-power validators retain `HashMap` iteration order before truncation by `max_validators`.
- **Why**: Violates deterministic epoch-transition rules (technical-patterns.md 4.1). Replicas with identical staking state can select different formal validator sets when a tie is truncated.
- **Suggested fix**: Sort by a total deterministic key, such as voting power descending then `ValidatorId` ascending.

---

### [HIGH] consensus: TC-carried highest QCs are not fully bound or validated
- **Where**: crates/hotmint-consensus/src/pacemaker.rs:203-223; crates/hotmint-types/src/certificate.rs:48-55; crates/hotmint-consensus/src/engine.rs:613-654,1238-1246
- **What**: Wish signatures bind only highest-QC view and hash, not epoch or aggregate signatures. `TimeoutCertificate::highest_qc()` selects the max QC from all entries, including unsigned or extra entries.
- **Why**: Violates message authentication and stale-QC handling invariants (technical-patterns.md 3.3, 3.4, 5.3). A TC can poison `state.highest_qc` with a QC not covered by a signer.
- **Suggested fix**: Bind a canonical QC digest in wish signing bytes, require `highest_qcs` to align with signer indexes, and validate the selected QC before updating state.

### [HIGH] consensus: fast-forward commit side effects can be lost after app-hash rejection
- **Where**: crates/hotmint-consensus/src/view_protocol.rs:396-426; crates/hotmint-consensus/src/engine.rs:1027-1055
- **What**: `on_proposal()` can run `try_commit()` and mutate application/commit state, then return `Err` on the proposed child block's `app_hash` mismatch. The engine then skips `process_commit_result()`.
- **Why**: Violates commit atomicity and persistence invariants (technical-patterns.md 6.1). A committed parent can be applied without commit side effects, state persistence, or WAL completion.
- **Suggested fix**: Validate the child before mutating commit state, or return/process the successful commit result even when the child proposal is rejected.

### [HIGH] consensus/storage: WAL intent fsync failures do not stop commits
- **Where**: crates/hotmint-consensus/src/engine.rs:1027-1034,1595-1606
- **What**: `wal.log_commit_intent()` errors are logged as warnings, but the engine continues into `try_commit()` and application execution.
- **Why**: Violates WAL durability and crash atomicity (technical-patterns.md 6.1). Disk-full or EIO can leave an applied commit with no durable intent for recovery.
- **Suggested fix**: Treat intent fsync failure as fatal before application execution; halt, return an error, or panic before committing.

### [HIGH] storage: WAL recovery detects incomplete commits but does not replay them
- **Where**: crates/hotmint-storage/src/wal.rs:88-95; crates/hotmint/src/bin/node.rs:326-337
- **What**: `ConsensusWal::check_recovery()` returns `NeedsReplay`, but startup only logs that the node will re-sync and continues.
- **Why**: Violates recovery correctness (technical-patterns.md 6.1). A crash after app commit but before state persistence can restart with the application ahead of consensus state.
- **Suggested fix**: Replay `last_committed_height + 1..=target_height` before engine start and persist state, or fail closed if replay cannot complete.

### [HIGH] sync/storage: sync replay commits application blocks without a durable checkpoint
- **Where**: crates/hotmint-consensus/src/sync.rs:560-620; crates/hotmint/src/bin/node.rs:672-835
- **What**: `replay_blocks()` stores blocks/QCs and calls `execute_block()`/`on_commit()`, but only updates in-memory sync state; startup later copies it into `state` without saving and flushing persistent consensus state.
- **Why**: Violates crash atomicity and block-store recovery invariants (technical-patterns.md 6.1, 6.2). A crash after sync replay can leave the app ahead of persisted consensus state.
- **Suggested fix**: Put sync replay under WAL discipline and persist/flush height, epoch, pending epoch, and app hash after each replayed block or batch.

### [HIGH] sync: snapshots are applied before authenticated state and epoch binding
- **Where**: crates/hotmint-consensus/src/sync.rs:246-410; crates/hotmint-types/src/sync.rs:53-57; crates/hotmint/src/bin/node.rs:772-835
- **What**: Snapshot chunks are applied before trust-anchor verification, `SnapshotInfo.hash` is unused, and successful snapshot sync updates height/app hash but not epoch, validator set, or pending epoch.
- **Why**: Violates sync convergence, state consistency, and epoch verification invariants (technical-patterns.md 4.1, 4.2, 6.1). A snapshot after validator changes can start the node with stale validator state.
- **Suggested fix**: Stage snapshots, verify chunk hash/restored app hash against a quorum-authenticated state commitment, and restore epoch/validator metadata with proof before mutating live state.

### [HIGH] consensus/storage: locked QC is not persisted before Vote2 emission
- **Where**: crates/hotmint-consensus/src/view_protocol.rs:520-551; crates/hotmint-consensus/src/engine.rs:1129-1135,1694-1708
- **What**: `on_prepare()` updates `locked_qc`/`highest_qc` and sends Vote2 before `persist_state()` is called later on view changes or commits.
- **Why**: Violates locked-QC and crash-safety invariants (technical-patterns.md 1.2, 6.1). A crash after Vote2 but before persistence can forget the lock and later vote on a conflicting branch.
- **Suggested fix**: Persist and flush the new lock before emitting Vote2; refactor `on_prepare()` to return the Vote2 after durable state save.

### [HIGH] consensus/evidence: evidence is acted on before durable persistence
- **Where**: crates/hotmint-consensus/src/engine.rs:1291-1344,1350-1366; crates/hotmint-consensus/src/commit.rs:150-160
- **What**: Gossiped and locally detected evidence calls `app.on_evidence()` and local detection broadcasts before `put_evidence()`/`flush()`.
- **Why**: Violates evidence durability (technical-patterns.md 1.1, 6.3). A crash during or after the callback can apply slashing while losing the proof.
- **Suggested fix**: Verify evidence, persist and flush it first, then notify the app and gossip; reserve state-mutating slashing for committed evidence or split precommit notifications from deterministic application.

### [HIGH] consensus/evidence: pending evidence is pruned without block inclusion
- **Where**: crates/hotmint-consensus/src/engine.rs:1568-1579; crates/hotmint-storage/src/evidence_store.rs:213-226
- **What**: `process_commit_result()` marks every pending proof with `proof.view <= block.view` as committed even if that proof is absent from `block.evidence`.
- **Why**: Violates evidence persistence and slashing liveness (technical-patterns.md 1.1, 6.3). Evidence omitted by a proposer or arriving after proposal creation can be deleted before deterministic inclusion.
- **Suggested fix**: Mark only proofs actually present in committed blocks; keep other pending proofs until inclusion or an explicit, deterministic expiry rule.

### [HIGH] storage/evidence: evidence `next_id` metadata is non-atomic and errors are swallowed
- **Where**: crates/hotmint-storage/src/evidence_store.rs:146-197
- **What**: `persist_next_id()` returns `()` and logs read/write/fsync failures. The sidecar counter is updated separately from the proof insert and before `vsdb_flush()`.
- **Why**: Violates evidence durability (technical-patterns.md 6.3). A crash or meta write failure can skip IDs, lose proofs, or overwrite durable evidence.
- **Suggested fix**: Store the counter transactionally with proofs or derive it from the proof map on open; make persistence fallible and propagate failures.

### [HIGH] types: Vote2 extensions are not signed
- **Where**: crates/hotmint-types/src/vote.rs:24-50; crates/hotmint-consensus/src/vote_collector.rs:86-99
- **What**: Vote extensions are carried on `Vote2Msg` and collected into double certificates, but `Vote::signing_bytes()` signs only chain, epoch, view, block hash, and vote type.
- **Why**: Violates payload-binding/domain-separation invariants (technical-patterns.md 7.1). A peer can modify extension bytes while reusing a valid Vote2 signature and falsely attribute application data to a validator.
- **Suggested fix**: Include an extension presence byte and extension hash/length in Vote2 signing bytes; reject extensions on first-phase votes.

### [HIGH] light: client does not maintain a verified hash-chain trust path
- **Where**: crates/hotmint-light/src/lib.rs:45-49,77-84,119-123
- **What**: `LightClient` stores trusted height and validator set but no trusted block hash. `verify_header()` checks only monotonic height and QC hash equality, and successful verification does not atomically advance trusted hash/height.
- **Why**: Violates committed-chain continuity (technical-patterns.md 1.4). This is distinct from the existing Won't Fix header-field binding issue: the client lacks a hash-chain checkpoint to advance from.
- **Suggested fix**: Track trusted `(height, hash, validator_set)`, require parent continuity or a verified skip proof, and make verification/update an atomic trust advancement.

### [HIGH] staking: unregistering removes slashable state while unbondings can mature
- **Where**: crates/hotmint-staking/src/manager.rs:77-92,199-202,256-271
- **What**: `unregister_validator()` removes validator and stake records immediately. Later `slash()` fails with "validator not found", so returned stake and pending unbondings escape evidence.
- **Why**: Violates evidence/slashing durability expectations (technical-patterns.md 6.3). A double-signing validator can unregister before evidence is processed.
- **Suggested fix**: Tombstone validators through the evidence/unbonding window, prevent unregister while slashable stake exists, and allow slashing tombstoned/pending stake.

### [HIGH] ABCI: ambiguous IPC retry can double-apply committed blocks
- **Where**: crates/hotmint-abci/src/client.rs:56-80,190-221
- **What**: `IpcApplicationClient::call()` retries every request after write/read errors. If `ExecuteBlock`, `OnCommit`, or `OnEvidence` reached the app but the response was lost, retry sends the non-idempotent request again.
- **Why**: Violates application/consensus crash consistency (technical-patterns.md 6.1). One consensus commit can be applied twice by an ABCI app.
- **Suggested fix**: Retry only before a complete request write is known safe, or add request IDs/height-based idempotency; fail closed on ambiguous post-write errors for non-idempotent calls.

### [HIGH] ABCI/sync: startup opens two IPC clients against a single-connection server
- **Where**: crates/hotmint-abci/src/client.rs:37-42; crates/hotmint-abci/src/server.rs:77-105; crates/hotmint/src/bin/node.rs:397-408,663-672
- **What**: `check_connection()` stores an idle primary UnixStream, then initial sync uses a second `IpcApplicationClient`. The server processes one connection at a time and blocks reading the idle stream.
- **Why**: Violates sync convergence (technical-patterns.md 2.3). An ABCI validator restarting behind peers can time out during initial catch-up because sync requests go to an unserved second connection.
- **Suggested fix**: Do not keep the health-check stream open before sync, share one client, or make the ABCI server handle concurrent connections safely.

### [HIGH] application/ABCI: wrappers drop consensus-critical `Application` callbacks
- **Where**: crates/hotmint/src/bin/node.rs:915-1014,1019-1048; crates/hotmint-abci/src/client.rs:119-234; crates/hotmint-consensus/src/engine.rs:297-318,1112-1158,1762-1781
- **What**: `AppWithStatus`, `ArcApp`, and `IpcApplicationClient` forward only part of the `Application` trait. `info()`, `init_chain`, offline-validator callbacks, vote extension callbacks, and snapshot callbacks fall back to defaults.
- **Why**: Violates recovery and application-contract invariants (technical-patterns.md 6.1). Startup divergence checks using `app.info()` are bypassed, and liveness/vote-extension/snapshot behavior is silently disabled.
- **Suggested fix**: Forward every `Application` method in wrappers and extend ABCI protocol support for engine-used callbacks, especially `Info`, or fail explicitly when unsupported.

### [HIGH] API/ABCI: unthrottled queries can monopolize the consensus ABCI client
- **Where**: crates/hotmint-api/src/rpc.rs:272-283,549-588; crates/hotmint-abci/src/client.rs:56-67
- **What**: Only `submit_tx` is rate-limited. Public `query` calls the same `Arc<dyn Application>` used by consensus; for ABCI it locks the same client mutex and performs blocking IPC.
- **Why**: Violates API resource-limit and view-progress invariants (technical-patterns.md 2.1, 5.2). Query floods can delay `execute_block()`/`on_commit()` and stall consensus.
- **Suggested fix**: Add per-IP/concurrency limits for all RPC methods, isolate blocking app calls, and use a separate bounded read-only query client or pool that cannot starve consensus callbacks.

### [HIGH] facade/network: generated node key does not match peer routing identity
- **Where**: crates/hotmint/src/config.rs:270-319,371-378,426-459; crates/hotmint/src/bin/node.rs:220-226,344-390; crates/hotmint-network/src/service.rs:272-285,486-495
- **What**: Nodes use independent `node_key.json` litep2p identities, but `parse_persistent_peers()` derives each `PeerId` from the validator public key because peers are configured as `<validator_id>@<multiaddr>`.
- **Why**: Violates peer-map consistency (technical-patterns.md 5.4). Normal multi-node configs dial and authenticate the wrong peer ID, causing consensus messages from actual node keys to be unknown or dropped.
- **Suggested fix**: Make node identity intentionally equal to validator identity everywhere, or extend genesis/config peer format with actual node `PeerId`/node public key and populate `PeerMap` from that mapping.

### [HIGH] network: large consensus messages are queued before ingress rate limiting
- **Where**: crates/hotmint-network/src/service.rs:45-47,296-300,491-496,572-575; crates/hotmint-consensus/src/engine.rs:917-938
- **What**: Inbound consensus notifications/requests are decoded and enqueued into an 8192-message shared channel before engine-side per-validator rate limiting and signature verification.
- **Why**: Violates bounded-queue and authentication-before-work invariants (technical-patterns.md 5.2, 5.3). One known peer can fill memory/queue slots with large invalid messages and drop honest traffic.
- **Suggested fix**: Apply byte-budgeted per-peer rate limits before decode/enqueue, reject unknown peers before decode, and verify/drop known-sender messages earlier where practical.

### [HIGH] network: `StatusCert` relay accepts unauthenticated messages
- **Where**: crates/hotmint-network/src/service.rs:504-529; crates/hotmint-consensus/src/engine.rs:575-600
- **What**: Relay validation returns true for `StatusCert` when the embedded validator is known, without checking `sender == validator` or verifying the status signature.
- **Why**: Violates message authentication for relay amplification (technical-patterns.md 5.3). A known peer can gossip fake status certificates and cause relays before the engine rejects them.
- **Suggested fix**: Do not relay status certificates, or require sender/validator match and verify the status signature before relay.

### [HIGH] network/mempool: mempool notification substreams are never opened
- **Where**: crates/hotmint-network/src/service.rs:248-255,865-908,997-1017
- **What**: The service registers a mempool notification protocol and broadcasts only to `mempool_notif_connected_peers`, but `ConnectionEstablished` opens only the consensus notification substream.
- **Why**: Violates network message propagation. Two normal nodes never populate `mempool_notif_connected_peers`, so `BroadcastTx` sends to nobody.
- **Suggested fix**: Open mempool notification substreams alongside consensus substreams and handle failures/cleanup symmetrically.

---

### [MEDIUM] storage: committed height index can point at a different same-height proposal
- **Where**: crates/hotmint-storage/src/block_store.rs:153-158; crates/hotmint-consensus/src/engine.rs:1551-1556; crates/hotmint/src/bin/node.rs:563-569
- **What**: `put_block()` overwrites `by_height[H]` for every proposal. If a different same-height proposal overwrites the index before another block commits, commit QCs remain keyed by height and can pair with the wrong block.
- **Why**: Violates block-store consistency (technical-patterns.md 6.2). Sync responders can serve block B with block A's commit QC.
- **Suggested fix**: Separate proposal storage from committed height indexing, re-pin `by_height` to the committed block, or key commit QCs by block hash.

### [MEDIUM] types/crypto: quorum threshold trusts inconsistent serialized total power
- **Where**: crates/hotmint-types/src/validator.rs:39-66,114-120; crates/hotmint-crypto/src/aggregate.rs:20-29
- **What**: `ValidatorSet` deserialization trusts serialized `total_power`, and construction sums powers without checked overflow.
- **Why**: Violates quorum threshold correctness (technical-patterns.md 3.1). A malformed or overflowed set can lower `quorum_threshold()` and make `has_quorum()` accept insufficient power.
- **Suggested fix**: Recompute and validate `total_power` during deserialization, make construction fallible with checked sums, and reject zero-total validator sets where consensus uses them.

### [MEDIUM] ABCI proto: consensus-critical fields are not strictly decoded
- **Where**: crates/hotmint-abci-proto/src/convert.rs:100-107,210-226,242-249,327-339; crates/hotmint-abci/src/protocol.rs:227-234
- **What**: Empty hashes decode as `BlockHash::GENESIS`, public keys/signatures accept arbitrary lengths, and a successful `ExecuteBlock` response with missing `result` becomes `EndBlockResponse::default()`.
- **Why**: Violates protobuf field-size validation and crypto strictness (technical-patterns.md 7.1). Malformed ABCI output can poison validator sets or treat an incomplete execution response as success.
- **Suggested fix**: Require exact 32-byte hashes/public keys and 64-byte signatures at conversion boundaries, and require `ExecuteBlock.result` when `error` is empty.

### [MEDIUM] API: WebSocket counter can leak on failed upgrade
- **Where**: crates/hotmint-api/src/http_rpc.rs:138-150,222-235
- **What**: The connection counter increments before `on_upgrade`, but decrement happens only inside `handle_ws()`. If the HTTP upgrade fails after the response is built, the RAII guard is never created.
- **Why**: Violates WebSocket resource cleanup. Repeated aborted upgrades can exhaust `MAX_WS_CONNECTIONS` until restart.
- **Suggested fix**: Use `on_failed_upgrade` to decrement or guard the entire upgrade lifecycle with a semaphore/permit.

### [MEDIUM] staking: already-jailed validators cannot receive later distinct slashes
- **Where**: crates/hotmint-staking/src/manager.rs:193-209
- **What**: `slash()` rejects any evidence for a validator already marked jailed.
- **Why**: Violates evidence-specific slashing semantics (technical-patterns.md 6.3). A low-penalty downtime slash can block a later high-penalty double-sign slash.
- **Suggested fix**: Track applied evidence IDs/reasons and permit distinct or more severe slashes while jailed.

### [MEDIUM] mempool: `collect_payload` removes transactions before commit finality
- **Where**: crates/hotmint-mempool/src/lib.rs:186-238; crates/hotmint-consensus/src/view_protocol.rs:153-164
- **What**: Proposal payload collection pops transactions from the pool and removes them from `seen` before the proposal commits.
- **Why**: Violates post-commit eviction semantics. If the leader times out or the proposal is rejected, transactions are lost locally or re-admitted while in flight.
- **Suggested fix**: Keep transactions pending until commit or track an in-flight set that is restored on failed views.

### [MEDIUM] mempool: gas accounting can overflow and bypass `max_gas`
- **Where**: crates/hotmint-mempool/src/lib.rs:200-228
- **What**: `total_gas + entry.gas_wanted` and `total_gas += entry.gas_wanted` use unchecked `u64` arithmetic.
- **Why**: Violates block gas boundary checks. Release builds can wrap and include over-budget transactions; debug builds can panic.
- **Suggested fix**: Use `checked_add` or `saturating_add` and skip entries whose gas addition overflows or exceeds `max_gas`.

### [MEDIUM] mempool: `max_size = 0` still admits one transaction
- **Where**: crates/hotmint-mempool/src/lib.rs:154-183; crates/hotmint/src/config.rs:89-93
- **What**: When `max_size` is zero, the full-pool branch sees `entries.first() == None` and continues to insert the transaction.
- **Why**: Violates configured pool-size limits. Operators cannot disable admission with a zero-sized pool.
- **Suggested fix**: Reject immediately when `self.max_size == 0`, or validate config to require `max_txs > 0`.

### [MEDIUM] network/mempool: gossip dedup marks transactions seen before enqueue succeeds
- **Where**: crates/hotmint-network/src/service.rs:300,1022-1050
- **What**: Inbound tx gossip inserts the tx hash into dedup before `mempool_tx_tx.try_send(...)`, and ignores `Full`.
- **Why**: Violates bounded-channel backpressure handling (technical-patterns.md 5.2). A full channel drops the tx but suppresses retries until dedup rotation.
- **Suggested fix**: Insert into dedup only after successful enqueue, or remove the hash on `TrySendError::Full`.

### [MEDIUM] network: consensus relay dedup can be bypassed with alternate wire encodings
- **Where**: crates/hotmint-network/src/service.rs:515-535; crates/hotmint-network/src/codec.rs:54-68
- **What**: Relay dedup hashes raw notification bytes. The same decoded consensus message can arrive as raw or zstd data, with multiple possible zstd encodings.
- **Why**: Violates replay/dedup expectations (technical-patterns.md 5.1). A valid signed message can be relayed repeatedly under alternate encodings.
- **Suggested fix**: Dedup on canonical message identity or canonical encoded message bytes, and relay canonical bytes.

### [MEDIUM] network/mempool: mempool substreams ignore chain-id handshake
- **Where**: crates/hotmint-network/src/service.rs:248-254,1007-1017,1022-1050
- **What**: Mempool notifications configure a chain-id handshake but auto-accept inbound streams and explicitly accept validation without checking handshake bytes.
- **Why**: Violates network chain-isolation boundaries. Cross-chain peers can open mempool streams, receive local tx broadcasts, and send tx gossip for validation.
- **Suggested fix**: Validate mempool notification handshakes exactly like consensus notifications and insert only validated peers.

### [MEDIUM] mgmt: stale process cleanup can kill unrelated processes
- **Where**: crates/hotmint-mgmt/src/lib.rs:167-180; crates/hotmint-mgmt/src/remote.rs:256-264,435-444
- **What**: Local cleanup uses `pkill -9 -f "--home <base_dir>"`; remote deployment uses predictable `/tmp/hotmint-v*.pid` files and `kill $(cat pid)` without validating the process.
- **Why**: Violates process-control safety. Broad pattern matches or tampered/reused PID files can terminate unrelated processes.
- **Suggested fix**: Store per-node PID files under controlled directories, validate PID ownership/executable/arguments before signaling, and avoid `pkill -f`.

---

### [LOW] facade/config: `hotmint-node init` writes a config that `hotmint-node node` rejects
- **Where**: crates/hotmint/src/config.rs:95-119,371-393; crates/hotmint/src/bin/node.rs:211-218; README.md:173-175; docs/getting-started.md:107-114
- **What**: `NodeConfig::default()` is validator mode with empty `proxy_app`, and `init_node_dir()` writes it. Startup rejects validator mode without `proxy_app`, while docs show `init` followed by `node`.
- **Why**: Violates configuration/doc-code alignment. The default documented path fails immediately.
- **Suggested fix**: Generate fullnode mode when `proxy_app` is empty, or generate/document a required proxy app for validators.

### [LOW] facade/docs: public facade docs overstate exports
- **Where**: crates/hotmint/README.md:8,23-32; crates/hotmint/src/lib.rs:39-77
- **What**: Docs claim the facade re-exports the entire ecosystem and prelude includes `Signature`/`AggregateSignature`, but the facade does not export `hotmint-light` and the prelude omits those signature types.
- **Why**: Violates public API/doc-code alignment. Users following docs get failed imports or incorrect expectations.
- **Suggested fix**: Add intended facade/prelude re-exports or narrow docs to the actual exports.

### [LOW] docs/application: snapshot default is documented opposite to code
- **Where**: docs/application.md:24-31; crates/hotmint-consensus/src/application.rs:204-211
- **What**: Documentation says default `apply_snapshot_chunk()` returns `ChunkApplyResult::Accept`, but code returns `ChunkApplyResult::Abort`.
- **Why**: Violates public API/doc-code alignment. Application authors can rely on the wrong default snapshot behavior.
- **Suggested fix**: Update docs to `Abort`, or intentionally change the trait default if accepting chunks is desired.

### [LOW] style: grouped import and inline-`std` conventions still have violations
- **Where**: crates/hotmint/src/bin/node.rs:9-32; crates/hotmint-mgmt/src/lib.rs:162,180,215
- **What**: `tokio::sync` imports are split across multiple imports, and `std::thread::sleep` is used inline three times in one file.
- **Why**: Violates project style rules in review-core.md and CLAUDE.md for grouped imports and repeated inline `std` paths.
- **Suggested fix**: Group the `tokio::sync` imports and import/use `std::thread::sleep` or `std::thread`.

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
