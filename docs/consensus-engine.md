# Consensus Engine

The `ConsensusEngine` is the heart of hotmint — an async event loop that drives the HotStuff-2 protocol.

## Overview

```rust
// SharedBlockStore = Arc<parking_lot::RwLock<Box<dyn BlockStore>>>
pub struct ConsensusEngine {
    state: ConsensusState,
    store: SharedBlockStore,
    network: Box<dyn NetworkSink>,
    app: Box<dyn Application>,
    signer: Box<dyn Signer>,
    verifier: Box<dyn Verifier>,
    vote_collector: VoteCollector,
    pacemaker: Pacemaker,
    pacemaker_config: PacemakerConfig,
    msg_rx: Receiver<(Option<ValidatorId>, ConsensusMessage)>,
    status_senders: HashSet<ValidatorId>,
    current_view_qc: Option<QuorumCertificate>,
    pending_epoch: Option<Epoch>,
    persistence: Option<Box<dyn StatePersistence>>,
    evidence_store: Option<Box<dyn EvidenceStore>>,
    liveness_tracker: LivenessTracker,
    wal: Option<Box<dyn Wal>>,
    msg_rate_limiter: HashMap<ValidatorId, (Instant, u32)>,
    /// Previous epoch's validator set, retained for verifying in-flight
    /// messages (especially TCs) formed before the epoch transition.
    previous_epoch: Option<Epoch>,
}
```

The engine takes ownership of all its dependencies and runs as an infinite async loop. It is `Send` and designed to be spawned onto a tokio runtime.

## Construction

```rust
use std::sync::Arc;
use parking_lot::RwLock;
use hotmint::consensus::engine::{ConsensusEngineBuilder, EngineConfig, SharedBlockStore};
use hotmint::crypto::Ed25519Verifier;
use hotmint::consensus::state::ConsensusState;

let store: SharedBlockStore = Arc::new(RwLock::new(Box::new(block_store)));

let engine = ConsensusEngineBuilder::new()
    .state(state)
    .store(store)
    .network(Box::new(network_sink))
    .app(Box::new(application))
    .signer(Box::new(signer))
    .messages(msg_rx)
    .verifier(Box::new(Ed25519Verifier))
    .build()
    .expect("all required fields must be set");
```

The `msg_rx` channel is the engine's sole input. All consensus messages — whether from the network or from loopback — arrive through this channel as `(Option<sender_id>, message)` tuples. The sender is `Some(ValidatorId)` for authenticated validators; `NetworkService` drops messages from unknown peers before they reach the channel, so `None` only ever comes from a custom sink that passes its own admission checks first.

## Running

```rust
// engine.run() consumes self and never returns
tokio::spawn(async move { engine.run().await });
```

The event loop:

```rust
loop {
    let deadline = self.pacemaker.sleep_until_deadline();
    tokio::pin!(deadline);

    tokio::select! {
        Some((sender, msg)) = self.msg_rx.recv() => {
            if let Err(e) = self.handle_message(sender, msg).await {
                warn!(error = %e, "error handling message");
            }
        }
        _ = &mut deadline => {
            self.handle_timeout().await;
        }
    }
}
```

## ConsensusState

The mutable state tracked by the engine:

```rust
pub struct ConsensusState {
    pub validator_id: ValidatorId,
    pub validator_set: ValidatorSet,
    /// Blake3 hash of the chain identifier — included in all signing bytes
    /// to prevent cross-chain signature replay.
    pub chain_id_hash: [u8; 32],
    pub current_view: ViewNumber,
    pub role: ViewRole,               // Leader or Replica
    pub step: ViewStep,               // progress within the current view
    pub locked_qc: Option<QuorumCertificate>,
    pub highest_double_cert: Option<DoubleCertificate>,
    pub highest_qc: Option<QuorumCertificate>,
    pub last_committed_height: Height,
    pub last_app_hash: BlockHash,     // state root after executing the most recently committed block
    pub current_epoch: Epoch,
    /// Vote extensions gathered for the next proposal (ABCI++).
    pub pending_vote_extensions: Vec<(ValidatorId, Vec<u8>)>,
}
```

`ConsensusState::new(validator_id, validator_set)` creates state with an empty chain ID (no domain separation). For production use, prefer `ConsensusState::with_chain_id(validator_id, validator_set, "my-chain")` which hashes the chain ID with Blake3 and stores it in `chain_id_hash`. This hash is included in all signing bytes to prevent cross-chain signature replay.

### Chain ID Domain Separator

The `chain_id_hash` field provides cross-chain replay prevention. When a chain ID is set, its Blake3 hash is included in the signing bytes of all consensus messages (votes, wishes, etc.). This means a signature produced for chain "alpha" is invalid on chain "beta", even if the same validator set is used on both chains.

```rust
// No chain ID (empty string, suitable for testing)
let state = ConsensusState::new(vid, validator_set.clone());

// With chain ID (recommended for production)
let state = ConsensusState::with_chain_id(vid, validator_set, "my-chain-id");
```

### ViewRole

```rust
pub enum ViewRole {
    Leader,   // proposes blocks, collects votes
    Replica,  // votes on proposals
}
```

The role is determined at view entry: `leader_for_view(v)` returns `Option<&ValidatorInfo>` (returns `None` only if the validator set is empty). In practice, use `.expect("non-empty validator set")` or `.unwrap()`:

```rust
if validator_set.leader_for_view(v).expect("non-empty validator set").id == self.validator_id {
    // this node is the leader
}
```

### ViewStep

Tracks progress through the view protocol:

```rust
pub enum ViewStep {
    Entered,             // just entered the view
    WaitingForStatus,    // leader: waiting for replica status messages
    Proposed,            // declared but never assigned — the leader moves to CollectingVotes
    WaitingForProposal,  // replica: waiting for leader's proposal
    Voted,               // replica: sent phase-1 vote
    CollectingVotes,     // leader: collecting phase-1 votes
    Prepared,            // leader: QC formed, Prepare sent
    SentVote2,           // replica: sent phase-2 vote
    Done,                // declared but never assigned — no code path marks a view complete
}
```

## Message Handling

Each `ConsensusMessage` variant is dispatched to a specific handler:

### Propose

```
Propose ──> view_protocol::on_proposal()
         ──> safety check: justify.rank >= locked_qc.rank
         ──> if safe: send VoteMsg to leader
```

The replica validates the block via `Application::validate_block()` before voting.

### VoteMsg (Phase 1)

```
VoteMsg ──> vote_collector::add_vote()
         ──> if quorum reached: on_qc_formed()
         ──> broadcast Prepare{QC}
```

The leader aggregates votes. Once the aggregate covers more than 2/3 of the voting power, a QC is formed and broadcast in a Prepare message.

### Prepare

```
Prepare ──> view_protocol::on_prepare()
         ──> update locked_qc to the received QC
         ──> send Vote2Msg to next view's leader
```

### Vote2Msg (Phase 2)

```
Vote2Msg ──> vote_collector::add_vote()
          ──> if quorum reached: on_double_cert_formed()
          ──> commit block and ancestors
          ──> advance to next view
```

### Wish

```
Wish ──> pacemaker::add_wish()
      ──> if quorum reached: form TimeoutCertificate
      ──> broadcast TC
      ──> advance view
```

### TimeoutCert

```
TimeoutCert ──> advance to view `tc.view + 1`
             ──> relay TC to other validators (if not seen before)
```

### StatusCert

```
StatusCert ──> leader collects status from replicas
            ──> when enough received: try_propose()
```

### Evidence

```
Evidence ──> look the accused validator up in the current validator set
         ──> reject if the two block hashes are identical
         ──> rebuild Vote::signing_bytes (using the proof's own epoch) and verify both signatures
         ──> persist in the EvidenceStore and flush immediately
```

Evidence arrives as `ConsensusMessage::Evidence(EquivocationProof)`. Anything that fails a
check — unknown validator, identical block hashes, invalid signatures — is dropped with a
warning and has no local effect. Evidence the node detects itself takes the other path
(`handle_equivocation`): store, flush, and `broadcast_evidence` to peers.

## Vote Collection

The `VoteCollector` manages vote aggregation for both phases:

```rust
pub struct VoteCollector {
    // (epoch, view, block_hash, vote_type) -> votes
}
```

When a quorum (more than 2/3 of the voting power) is reached:
- Phase 1: forms a `QuorumCertificate` with an `AggregateSignature`
- Phase 2: forms the outer QC; the engine pairs it with the view's inner QC to assemble the `DoubleCertificate`

The collector prunes stale votes for old views to prevent memory growth.

## Commit Process

When a double certificate is formed:

1. Identify the committed block from the double certificate
2. Walk the chain from the committed block backward to `last_committed_height + 1`
3. **WAL: log commit intent** (if WAL is configured)
4. For each block in ascending height order:
   - `app.on_evidence(proof)` once per embedded proof, before anything else
   - Decode payload into transactions
   - `app.execute_block(txs, ctx)` (where `txs` is `&[&[u8]]` and `ctx` is a `BlockContext` with height, view, proposer, epoch, epoch_start_view, validator_set, timestamp, vote_extensions; returns `EndBlockResponse` which may contain validator updates, events, and app_hash)
   - `app.on_commit(block, ctx)`
   - Store commit QC, tx index, and block results in the block store
   - Record the commit QC's signer bitfield in the `LivenessTracker` (one sample per DoubleCertificate) for offline detection
5. Update `last_committed_height` and persist consensus state
6. **WAL: log commit done** (triggers WAL truncation)
7. At epoch boundaries: query `LivenessTracker::offline_validators()` and call `app.on_offline_validators()`

## Pacemaker Integration

The pacemaker manages view timeouts independently of message processing:

- **Base timeout**: 2 seconds (`BASE_TIMEOUT_MS`)
- **Backoff**: 1.5× per consecutive timeout, capped at 30 seconds (`MAX_TIMEOUT_MS`)
- **Reset**: only when a view advance is driven by a `DoubleCert` (`Pacemaker::reset_on_progress`). TC-driven advances call `reset_timer()`, which restarts the timer but keeps the consecutive-timeout count, so backoff survives repeated view changes.

On timeout, the engine:
1. Builds and broadcasts a `Wish` message
2. Applies exponential backoff to the timer
3. Continues listening for messages (the view is not abandoned until a TC forms)

See [Protocol](protocol.md) for the full pacemaker specification.

## Signal Handling (Graceful Shutdown)

The `hotmint` node binary handles SIGINT (Ctrl+C) and SIGTERM for graceful shutdown. The main `tokio::select!` block races the engine/network tasks against signal handlers:

```rust
tokio::select! {
    // ... engine and network tasks ...
    _ = tokio::signal::ctrl_c() => {
        info!("received shutdown signal, exiting...");
    }
    _ = async {
        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::terminate()
            ).expect("failed to register SIGTERM handler");
            sigterm.recv().await;
        }
    } => {
        info!("received SIGTERM, shutting down...");
    }
}
```

When either signal is received, the select block completes and the process exits cleanly. This is handled at the binary level (`crates/hotmint/src/bin/node.rs`), not inside the `ConsensusEngine` itself.
