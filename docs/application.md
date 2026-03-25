# Application

The `Application` trait is hotmint's equivalent of Tendermint's ABCI (Application BlockChain Interface). It defines the boundary between the consensus engine and your business logic.

## Trait Definition

```rust
pub trait Application: Send + Sync {
    // Startup & genesis
    fn info(&self) -> AppInfo                                           { AppInfo::default() }
    fn init_chain(&self, _app_state: &[u8]) -> Result<BlockHash>       { Ok(BlockHash::default()) }
    // Block lifecycle
    fn create_payload(&self, _ctx: &BlockContext) -> Vec<u8>            { vec![] }
    fn validate_block(&self, _block: &Block, _ctx: &BlockContext) -> bool { true }
    fn validate_tx(&self, _tx: &[u8], _ctx: Option<&TxContext>) -> TxValidationResult { TxValidationResult::accept(0) }
    fn execute_block(&self, _txs: &[&[u8]], _ctx: &BlockContext) -> Result<EndBlockResponse> { Ok(EndBlockResponse::default()) }
    fn on_commit(&self, _block: &Block, _ctx: &BlockContext) -> Result<()> { Ok(()) }
    // Evidence & liveness
    fn on_evidence(&self, _proof: &EquivocationProof) -> Result<()>    { Ok(()) }
    fn on_offline_validators(&self, _offline: &[OfflineEvidence]) -> Result<()> { Ok(()) }
    // Vote extensions (ABCI++)
    fn extend_vote(&self, _block: &Block, _ctx: &BlockContext) -> Option<Vec<u8>> { None }
    fn verify_vote_extension(&self, _ext: &[u8], _block_hash: &BlockHash, _validator: ValidatorId) -> bool { true }
    // State sync snapshots
    fn list_snapshots(&self) -> Vec<SnapshotInfo>                       { vec![] }
    fn load_snapshot_chunk(&self, _height: Height, _chunk_index: u32) -> Vec<u8> { vec![] }
    fn offer_snapshot(&self, _snapshot: &SnapshotInfo) -> SnapshotOfferResult { SnapshotOfferResult::Reject }
    fn apply_snapshot_chunk(&self, _chunk: Vec<u8>, _chunk_index: u32) -> ChunkApplyResult { ChunkApplyResult::Accept }
    // Queries & config
    fn query(&self, _path: &str, _data: &[u8]) -> Result<QueryResponse> { Ok(QueryResponse::default()) }
    fn tracks_app_hash(&self) -> bool                                   { true }
}
```

All methods have default no-op implementations, so you only need to implement the ones your application cares about.

### BlockContext

Most `Application` methods receive a `BlockContext` that provides full consensus context:

```rust
pub struct BlockContext<'a> {
    pub height: Height,
    pub view: ViewNumber,
    pub proposer: ValidatorId,
    pub epoch: EpochNumber,
    pub epoch_start_view: ViewNumber,
    pub validator_set: &'a ValidatorSet,
    /// Aggregated vote extensions from previous round's Vote2 messages.
    pub vote_extensions: Vec<(ValidatorId, Vec<u8>)>,
}
```

This gives the application access to the current epoch number, the active validator set, and vote extensions from the previous round without maintaining separate state.

### TxContext

`validate_tx` receives an optional `TxContext` providing lightweight context for mempool validation:

```rust
pub struct TxContext {
    pub height: Height,
    pub epoch: EpochNumber,
}
```

When called from the mempool (pre-consensus), `ctx` is `Some` with the current chain tip info. During block re-validation it may be `None`.

## Lifecycle

When a block is committed, the consensus engine invokes the application methods in this order:

```
execute_block(txs, ctx)  →  EndBlockResponse { validator_updates, events }
    │
on_commit(block, ctx)
```

The payload in the committed block is decoded into individual transactions (using `Mempool::decode_payload`), and all transactions are passed at once to `execute_block` as `&[&[u8]]` along with the current `BlockContext`. This single-call design replaces the old three-step `begin_block` / `deliver_tx` / `end_block` lifecycle, enabling more efficient implementations (batch DB writes, parallel signature verification, etc.).

If `execute_block` returns an `EndBlockResponse` with non-empty `validator_updates`, an epoch transition is scheduled. The new validator set takes effect at the next view boundary.

## Method Reference

### `on_commit(block: &Block, ctx: &BlockContext) -> Result<()>`

Called after a block is finalized and `execute_block` has completed. Has a default no-op implementation. Override this to perform post-commit actions (emit events, update caches, etc.). The `ctx` provides the current epoch and validator set.

```rust
fn on_commit(&self, block: &Block, ctx: &BlockContext) -> Result<()> {
    self.db.apply_block(block.height.as_u64(), &block.payload)?;
    self.event_bus.emit(BlockCommitted {
        height: block.height,
        epoch: ctx.epoch,
    });
    Ok(())
}
```

### `create_payload(ctx: &BlockContext) -> Vec<u8>`

Called when this validator is the **leader** and needs to propose a new block. Return the block payload — typically serialized transactions collected from the mempool.

```rust
fn create_payload(&self, _ctx: &BlockContext) -> Vec<u8> {
    // collect up to 1MB of pending transactions
    let rt = tokio::runtime::Handle::current();
    rt.block_on(self.mempool.collect_payload(1_048_576))
}
```

If you return an empty `Vec<u8>`, the leader proposes an empty block.

### `validate_block(block: &Block, ctx: &BlockContext) -> bool`

Called by **replicas** before voting on a proposed block. Return `false` to reject the proposal (the replica will not vote). The `ctx` provides the current epoch and validator set for context-aware validation.

```rust
fn validate_block(&self, block: &Block, _ctx: &BlockContext) -> bool {
    // reject blocks with oversized payloads
    if block.payload.len() > 2_097_152 {
        return false;
    }
    // verify all transactions in the payload
    let txs = hotmint::mempool::Mempool::decode_payload(&block.payload);
    txs.iter().all(|tx| self.validate_tx(tx, None))
}
```

### `validate_tx(tx: &[u8], ctx: Option<&TxContext>) -> TxValidationResult`

Validate an individual transaction. Used by the mempool to filter incoming transactions before they enter the pool. Returns a `TxValidationResult` containing validation status, priority (for mempool ordering), and gas estimate. The optional `TxContext` provides the current height and epoch when available (e.g., during mempool admission); it may be `None` during block re-validation.

```rust
pub struct TxValidationResult {
    pub valid: bool,
    pub priority: u64,     // higher = included first
    pub gas_wanted: u64,   // gas estimate for block gas limit
}

impl TxValidationResult {
    pub fn accept(priority: u64) -> Self;
    pub fn accept_with_gas(priority: u64, gas_wanted: u64) -> Self;
    pub fn reject() -> Self;
}

fn validate_tx(&self, tx: &[u8], ctx: Option<&TxContext>) -> TxValidationResult {
    if MyTransaction::decode(tx).is_ok() {
        TxValidationResult::accept(0)
    } else {
        TxValidationResult::reject()
    }
}
```

### `execute_block(txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse>`

Called once per committed block with all decoded transactions at once. This is the primary method for applying block state transitions. The `ctx` provides the block's height, view, proposer, epoch, and validator set. Return an `EndBlockResponse` to optionally trigger an epoch transition with validator set updates.

Receiving all transactions in a single call enables efficient patterns such as batch DB writes, parallel signature verification, or pre-sorted execution.

```rust
fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
    let mut state = self.state.lock();
    state.begin_tx_batch(ctx.height.as_u64());
    for tx_bytes in txs {
        let tx = MyTransaction::decode(tx_bytes).map_err(|e| eg!("decode: {e}"))?;
        state.execute(tx)?;
    }
    state.commit_tx_batch(ctx.height.as_u64());
    Ok(EndBlockResponse::default()) // no validator changes
}
```

### `query(path: &str, data: &[u8]) -> Result<QueryResponse>`

Handle read-only queries from the JSON-RPC API. Returns a `QueryResponse` with the result data, optional Merkle proof, and the height at which the query was evaluated.

```rust
pub struct QueryResponse {
    pub data: Vec<u8>,
    pub proof: Option<Vec<u8>>,  // optional Merkle proof
    pub height: u64,
}

fn query(&self, path: &str, data: &[u8]) -> Result<QueryResponse> {
    match path {
        "balance" => {
            let addr = String::from_utf8_lossy(data);
            let balance = self.state.lock().get_balance(&addr);
            Ok(QueryResponse {
                data: balance.to_le_bytes().to_vec(),
                proof: None,
                height: self.last_height(),
            })
        }
        _ => Err(eg!("unknown query: {path}")),
    }
}
```

### `tracks_app_hash() -> bool`

Returns whether this application produces and verifies `app_hash` state roots. Defaults to `true`.

Applications that do not maintain a deterministic state root (e.g. `NoopApplication` used by fullnodes without an ABCI backend) should return `false`. When `false`, sync bypasses the `app_hash` equality check and accepts the chain's authoritative value, allowing the node to follow a chain produced by peers running a real application.

```rust
fn tracks_app_hash(&self) -> bool {
    true // override to false for stateless / observer nodes
}
```

### `on_evidence(proof: &EquivocationProof) -> Result<()>`

Called when equivocation (double-voting) is detected by the VoteCollector. A validator that votes for two different blocks in the same (view, vote_type) produces an `EquivocationProof`. The application can use this callback to implement slashing logic.

```rust
fn on_evidence(&self, proof: &EquivocationProof) -> Result<()> {
    tracing::warn!(
        validator = ?proof.validator,
        view = proof.view.as_u64(),
        "equivocation detected — slashing"
    );
    self.slashing_state.lock().slash(proof.validator, proof.view);
    Ok(())
}
```

The `EquivocationProof` contains:

```rust
pub struct EquivocationProof {
    pub validator: ValidatorId,
    pub view: ViewNumber,
    pub vote_type: VoteType,
    pub block_hash_a: BlockHash,
    pub signature_a: Signature,
    pub block_hash_b: BlockHash,
    pub signature_b: Signature,
}
```

Both signatures are retained so that any third party can independently verify the proof.

### `on_offline_validators(offline: &[OfflineEvidence]) -> Result<()>`

Called at epoch boundaries with validators detected as offline by the `LivenessTracker`. The application can use this to slash for downtime via `hotmint-staking`:

```rust
fn on_offline_validators(&self, offline: &[OfflineEvidence]) -> Result<()> {
    for ev in offline {
        tracing::warn!(validator = ?ev.validator, missed = ev.missed_commits, "offline — slashing");
        self.staking.slash(ev.validator, SlashReason::Downtime, ev.evidence_height);
    }
    Ok(())
}
```

The `OfflineEvidence` contains `validator`, `missed_commits`, `total_commits`, and `evidence_height`.

### `info() -> AppInfo`

Called at startup to reconcile application state with consensus state. Returns the last committed height and app hash:

```rust
fn info(&self) -> AppInfo {
    AppInfo {
        last_block_height: self.db.latest_height(),
        last_block_app_hash: self.db.latest_app_hash(),
    }
}
```

### `init_chain(app_state: &[u8]) -> Result<BlockHash>`

Called once before the first block during chain genesis. Initialize application state from the genesis data and return the initial app hash:

```rust
fn init_chain(&self, app_state: &[u8]) -> Result<BlockHash> {
    let genesis: MyGenesis = serde_json::from_slice(app_state)?;
    self.db.apply_genesis(&genesis)?;
    Ok(self.db.compute_app_hash())
}
```

### `extend_vote(block: &Block, ctx: &BlockContext) -> Option<Vec<u8>>`

Called before sending a Vote2 message. Return application-specific data to attach to the vote (ABCI++ vote extensions). Useful for oracles, threshold signatures, or MEV mitigation:

```rust
fn extend_vote(&self, _block: &Block, _ctx: &BlockContext) -> Option<Vec<u8>> {
    Some(self.oracle.latest_price_feed().encode())
}
```

### `verify_vote_extension(ext: &[u8], block_hash: &BlockHash, validator: ValidatorId) -> bool`

Called when receiving a Vote2 with an extension. Return `false` to reject the vote:

```rust
fn verify_vote_extension(&self, ext: &[u8], _block_hash: &BlockHash, _validator: ValidatorId) -> bool {
    OraclePriceFeed::decode(ext).is_ok()
}
```

### Snapshot Methods (State Sync)

Four methods support snapshot-based state sync for new nodes:

```rust
fn list_snapshots(&self) -> Vec<SnapshotInfo>;
fn load_snapshot_chunk(&self, height: Height, chunk_index: u32) -> Vec<u8>;
fn offer_snapshot(&self, snapshot: &SnapshotInfo) -> SnapshotOfferResult;
fn apply_snapshot_chunk(&self, chunk: Vec<u8>, chunk_index: u32) -> ChunkApplyResult;
```

See [P1-1 in the roadmap](security-audit-and-roadmap.md) for implementation details.

## EndBlockResponse and Epoch Transitions

`execute_block` returns an `EndBlockResponse` that can trigger dynamic validator set changes:

```rust
pub struct EndBlockResponse {
    pub validator_updates: Vec<ValidatorUpdate>,
    pub events: Vec<Event>,
    pub app_hash: BlockHash,  // application state root after executing this block
}

pub struct ValidatorUpdate {
    pub id: ValidatorId,
    pub public_key: PublicKey,
    pub power: u64, // power = 0 means remove the validator
}

pub struct Event {
    pub r#type: String,
    pub attributes: Vec<EventAttribute>,
}

pub struct EventAttribute {
    pub key: String,
    pub value: String,
}
```

When `validator_updates` is non-empty, the consensus engine schedules an **epoch transition**. The `events` field allows the application to emit structured events that external consumers (indexers, UIs) can observe.

1. The engine applies the updates to the current `ValidatorSet` to produce a new set.
2. A new `Epoch` is constructed with an incremented `EpochNumber` and the updated validator set.
3. The transition takes effect at the next view boundary (in `advance_view_to`).
4. The new epoch is persisted via `PersistentConsensusState::save_current_epoch`.

Example: removing a slashed validator at a specific block height:

```rust
fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
    // process transactions
    for tx in txs {
        self.apply_tx(tx, ctx)?;
    }

    // check for slashed validators
    let slashed = self.slashing_state.lock().drain_slashed();
    if slashed.is_empty() {
        return Ok(EndBlockResponse::default());
    }
    let updates = slashed
        .into_iter()
        .map(|id| {
            let info = ctx.validator_set.get(&id).unwrap();
            ValidatorUpdate {
                id,
                public_key: info.public_key.clone(),
                power: 0, // remove
            }
        })
        .collect();
    Ok(EndBlockResponse { validator_updates: updates, ..Default::default() })
}
```

## Complete Example

A key-value store application with transaction support:

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use ruc::*;
use hotmint::prelude::*;
use hotmint::consensus::application::Application;

struct KvStoreApp {
    store: Mutex<HashMap<String, String>>,
}

impl KvStoreApp {
    fn new() -> Self {
        Self { store: Mutex::new(HashMap::new()) }
    }
}

impl Application for KvStoreApp {
    fn validate_tx(&self, tx: &[u8], _ctx: Option<&TxContext>) -> TxValidationResult {
        // expect "key=value" format
        let s = String::from_utf8_lossy(tx);
        if s.contains('=') && s.split('=').count() == 2 {
            TxValidationResult::accept(0)
        } else {
            TxValidationResult::reject()
        }
    }

    fn validate_block(&self, block: &Block, _ctx: &BlockContext) -> bool {
        let txs = hotmint::mempool::Mempool::decode_payload(&block.payload);
        txs.iter().all(|tx| self.validate_tx(tx, None).valid)
    }

    fn execute_block(&self, txs: &[&[u8]], _ctx: &BlockContext) -> Result<EndBlockResponse> {
        let mut store = self.store.lock().unwrap();
        for tx in txs {
            let s = String::from_utf8_lossy(tx);
            let mut parts = s.splitn(2, '=');
            let key = parts.next().ok_or_else(|| eg!("missing key"))?.to_string();
            let val = parts.next().ok_or_else(|| eg!("missing value"))?.to_string();
            store.insert(key, val);
        }
        Ok(EndBlockResponse::default())
    }

    fn on_commit(&self, block: &Block, _ctx: &BlockContext) -> Result<()> {
        let store = self.store.lock().unwrap();
        println!(
            "height={} entries={} hash={}",
            block.height.as_u64(),
            store.len(),
            block.hash
        );
        Ok(())
    }

    fn query(&self, path: &str, data: &[u8]) -> Result<QueryResponse> {
        match path {
            "get" => {
                let key = String::from_utf8_lossy(data);
                let store = self.store.lock().unwrap();
                let val = store.get(key.as_ref()).map(|v| v.as_bytes().to_vec()).unwrap_or_default();
                Ok(QueryResponse { data: val, proof: None, height: 0 })
            }
            _ => Err(eg!("unknown query: {path}")),
        }
    }
}
```

## NoopApplication

For testing or when you don't need application logic. `NoopApplication` overrides `tracks_app_hash` to return `false` so that fullnodes without a real application backend can sync without failing the `app_hash` check:

```rust
pub struct NoopApplication;

impl Application for NoopApplication {
    fn tracks_app_hash(&self) -> bool {
        false
    }
}
```

Usage:

```rust
use hotmint::consensus::application::NoopApplication;

use std::sync::{Arc, RwLock};
use hotmint::consensus::engine::{EngineConfig, SharedBlockStore};
use hotmint::crypto::Ed25519Verifier;

let shared_store: SharedBlockStore = Arc::new(RwLock::new(Box::new(store)));
let engine = ConsensusEngine::new(
    state,
    shared_store,
    Box::new(network),
    Box::new(NoopApplication),
    Box::new(signer),
    msg_rx,
    EngineConfig {
        verifier: Box::new(Ed25519Verifier),
        pacemaker: None,
        persistence: None,
    },
);
```
