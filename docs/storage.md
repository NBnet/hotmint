# Storage

Hotmint provides two `BlockStore` implementations and a `PersistentConsensusState` for block persistence and consensus state crash recovery, respectively.

| Component | Purpose | Backend |
|:----------|:--------|:--------|
| `MemoryBlockStore` | Development / testing | HashMap + BTreeMap |
| `VsdbBlockStore` | Production | vsdb MapxOrd |
| `PersistentConsensusState` | Consensus state crash recovery | vsdb MapxOrd |
| `ConsensusWal` | Write-ahead log for commit crash recovery | File I/O |
| `PersistentEvidenceStore` | Equivocation proof persistence | vsdb MapxOrd |

## vsdb Overview

[vsdb](https://crates.io/crates/vsdb) is a high-performance embedded key-value database whose API mirrors Rust standard collections (HashMap / BTreeMap). Under the hood it uses MMDB, a pure-Rust LSM-Tree storage engine, so there are no C library dependencies.

Hotmint pins `vsdb = "16.3.9"`.

### Core Types

Core vsdb v16.x types used by Hotmint:

| Type | Description | Rust Equivalent |
|:-----|:------------|:----------------|
| `MapxOrd<K, V>` | Ordered KV store | `BTreeMap<K, V>` |
| `Mapx<K, V>` | Unordered KV store | `HashMap<K, V>` |
| `Orphan<T>` | Single-value persistent container | `Box<T>` on disk |

Common `MapxOrd` methods:

```rust
// Create (a namespace-scoped handle; Hotmint uses `MapxOrd::new_in(&Namespace::default_ns())`)
let mut map: MapxOrd<u64, String> = MapxOrd::new();

// Write
map.insert(&1, &"hello".into());

// Read
let val: Option<String> = map.get(&1);
let exists: bool = map.contains_key(&1);

// Range queries
let first: Option<(u64, String)> = map.first();
let last: Option<(u64, String)> = map.last();
let le: Option<(u64, String)> = map.get_le(&5);  // last entry ≤ 5
let ge: Option<(u64, String)> = map.get_ge(&5);  // first entry ≥ 5

// Iteration
for (k, v) in map.iter() { /* ... */ }
for (k, v) in map.range(10..20) { /* ... */ }

// Delete
map.remove(&1);
map.clear();
```

### Serialization Requirements

- Values must implement `ValueEnDe` — any `Serialize + Deserialize` type does, via a blanket impl
- Keys must implement `KeyEnDeOrdered` (ordered encoding), which is **not** blanket-implemented for arbitrary serde types: `String`, `RawBytes`/`Box<[u8]>`, integers, and integer arrays/vecs have built-in ordered encodings, and a custom key type needs an explicit `KeyEnDeOrdered` impl

### Key Functions

```rust
// Set the data directory (must be called before any vsdb operation; can only be called once)
vsdb::vsdb_set_base_dir("/var/lib/hotmint/data").unwrap();

// Get the current data directory
let dir = vsdb::vsdb_get_base_dir();

// Force flush to disk
vsdb::vsdb_flush();
```

## BlockStore Trait

```rust
pub trait BlockStore: Send + Sync {
    fn put_block(&mut self, block: Block);
    fn get_block(&self, hash: &BlockHash) -> Option<Block>;
    fn get_block_by_height(&self, h: Height) -> Option<Block>;

    // Commit QC storage (for light client queries)
    fn put_commit_qc(&mut self, _height: Height, _qc: QuorumCertificate) {}
    fn get_commit_qc(&self, _height: Height) -> Option<QuorumCertificate> { None }

    // Transaction indexing (tx_hash → (height, index_in_block))
    fn put_tx_index(&mut self, _tx_hash: [u8; 32], _height: Height, _index: u32) {}
    fn get_tx_location(&self, _tx_hash: &[u8; 32]) -> Option<(Height, u32)> { None }

    // Block execution results (events, app_hash)
    fn put_block_results(&mut self, _height: Height, _results: EndBlockResponse) {}
    fn get_block_results(&self, _height: Height) -> Option<EndBlockResponse> { None }

    /// Get blocks in [from, to] inclusive. Default iterates one-by-one.
    fn get_blocks_in_range(&self, from: Height, to: Height) -> Vec<Block> { /* default */ }

    /// Return the highest stored block height. Default returns genesis.
    fn tip_height(&self) -> Height { Height::GENESIS }

    /// Flush pending writes to durable storage.
    fn flush(&self) {}
}
```

The trait returns owned `Block` values (not references) because vsdb stores data on disk and cannot hand out borrowed references into memory. This design lets in-memory and persistent implementations share the same interface.

## MemoryBlockStore

An in-memory implementation suited for testing, development, and short-lived processes.

```rust
use hotmint::consensus::store::MemoryBlockStore;

let store = MemoryBlockStore::new();
// Automatically includes the genesis block at height 0
```

For convenience, a thread-safe shared instance can be created in one step:

```rust
let shared_store = MemoryBlockStore::new_shared();
// Returns Arc<RwLock<Box<dyn BlockStore>>>
```

Internal structure:
- `by_hash: HashMap<BlockHash, Block>` — O(1) hash lookup
- `by_height: BTreeMap<u64, BlockHash>` — ordered height lookup
- `commit_qcs: HashMap<u64, QuorumCertificate>` — commit QC by height

## VsdbBlockStore

A persistent block store backed by vsdb `MapxOrd`. Blocks survive process restarts.

```rust
use hotmint::storage::block_store::VsdbBlockStore;

// Production: call vsdb_set_base_dir(data_dir) first; opens or creates
// data_dir/block_store.meta so the collections are recovered on restart.
let store = VsdbBlockStore::open(&data_dir)?;

// Test-only: a fresh in-memory store with no meta file — nothing from a
// previous run is visible.
// let store = VsdbBlockStore::new();

// Both constructors seed the store with the genesis block on creation.

// Check whether a block exists
if store.contains(&block_hash) {
    // ...
}

// Explicitly flush to disk
store.flush();
```

### Internal Data Model

```rust
pub struct VsdbBlockStore {
    by_hash: MapxOrd<[u8; 32], Block>,              // BlockHash → Block
    by_height: MapxOrd<u64, [u8; 32]>,              // Height → BlockHash
    commit_qcs: MapxOrd<u64, QuorumCertificate>,    // Height → commit QC
    tx_index: MapxOrd<[u8; 32], (u64, u32)>,        // tx_hash → (height, index_in_block)
    block_results: MapxOrd<u64, EndBlockResponse>,   // Height → execution results
}
```

`open()` persists the five collections' instance IDs in `data_dir/block_store.meta` (five little-endian `u64` map IDs, 40 bytes) and restores them with `MapxOrd::from_meta` on the next open; a meta file that is not 40 bytes is rejected as corrupt rather than migrated. The five indexes work together:
- `put_block()` writes to both maps
- `get_block()` looks up directly in `by_hash`
- `get_block_by_height()` resolves the hash via `by_height`, then fetches the block from `by_hash`

### Using with ConsensusEngine

```rust
use std::sync::Arc;
use parking_lot::RwLock;
use hotmint::consensus::engine::{ConsensusEngineBuilder, SharedBlockStore};
use hotmint::crypto::Ed25519Verifier;

let store: SharedBlockStore =
    Arc::new(RwLock::new(Box::new(VsdbBlockStore::open(&data_dir)?)));

let engine = ConsensusEngineBuilder::new()
    .state(state)
    .store(store)                           // SharedBlockStore = Arc<RwLock<Box<dyn BlockStore>>>
    .network(Box::new(network_sink))        // Box<dyn NetworkSink>
    .app(Box::new(app))                     // Box<dyn Application>
    .signer(Box::new(signer))              // Box<dyn Signer>
    .messages(msg_rx)                       // Receiver<(Option<ValidatorId>, ConsensusMessage)>
    .verifier(Box::new(Ed25519Verifier))   // Box<dyn Verifier>
    .build()
    .expect("all required fields must be set");
```

## PersistentConsensusState

Critical consensus state (view number, locked QC, highest QC, committed height, current epoch) must be recovered after a crash to maintain safety.

### Internal Data Model

```rust
// Multiple state fields stored in a single MapxOrd
pub struct PersistentConsensusState {
    store: MapxOrd<u64, StateValue>,
}

// State value enum (serialized via serde)
enum StateValue {
    View(ViewNumber),
    Height(Height),
    Qc(QuorumCertificate),
    Epoch(Epoch),
    AppHash(BlockHash),
}

// Fixed key constants
const KEY_CURRENT_VIEW: u64 = 1;
const KEY_LOCKED_QC: u64 = 2;
const KEY_HIGHEST_QC: u64 = 3;
const KEY_LAST_COMMITTED_HEIGHT: u64 = 4;
const KEY_CURRENT_EPOCH: u64 = 5;
const KEY_LAST_APP_HASH: u64 = 6;
const KEY_PENDING_EPOCH: u64 = 7;    // in-flight epoch transition (crash recovery)
const KEY_PREVIOUS_EPOCH: u64 = 8;   // previous epoch, kept for verifying in-flight messages
```

### API

```rust
use hotmint::storage::consensus_state::PersistentConsensusState;

// Production: requires vsdb_set_base_dir(data_dir) first; `new()` is the
// test-only in-memory constructor.
let mut pstate = PersistentConsensusState::open(&data_dir)?;

// Save state (typically called after view changes or commits)
pstate.save_current_view(ViewNumber(42));
pstate.save_locked_qc(&qc);
pstate.save_highest_qc(&highest_qc);
pstate.save_last_committed_height(Height(10));
pstate.save_current_epoch(&epoch);
pstate.save_last_app_hash(app_hash);
pstate.save_pending_epoch(Some(&pending_epoch));     // in-flight epoch transition
pstate.save_previous_epoch(Some(&previous_epoch));   // previous epoch's validator set
pstate.flush();

// Load state (at startup / crash recovery)
let view = pstate.load_current_view();           // Option<ViewNumber>
let locked = pstate.load_locked_qc();            // Option<QuorumCertificate>
let highest = pstate.load_highest_qc();          // Option<QuorumCertificate>
let committed = pstate.load_last_committed_height(); // Option<Height>
let epoch = pstate.load_current_epoch();         // Option<Epoch>
let app_hash = pstate.load_last_app_hash();      // Option<BlockHash>
let pending = pstate.load_pending_epoch();       // Option<Epoch>
let previous = pstate.load_previous_epoch();     // Option<Epoch>
```

### Crash Recovery Example

```rust
use hotmint::consensus::state::ConsensusState;
use hotmint::storage::block_store::VsdbBlockStore;
use hotmint::storage::consensus_state::PersistentConsensusState;

fn recover_or_init(vid: ValidatorId, vs: ValidatorSet) -> (ConsensusState, VsdbBlockStore) {
    let store = VsdbBlockStore::open(&data_dir)?;
    let pstate = PersistentConsensusState::open(&data_dir)?;

    let mut state = ConsensusState::new(vid, vs);

    // Restore from persisted state
    if let Some(view) = pstate.load_current_view() {
        state.current_view = view;
    }
    if let Some(qc) = pstate.load_locked_qc() {
        state.locked_qc = Some(qc);
    }
    if let Some(qc) = pstate.load_highest_qc() {
        state.highest_qc = Some(qc);
    }
    if let Some(h) = pstate.load_last_committed_height() {
        state.last_committed_height = h;
    }
    if let Some(epoch) = pstate.load_current_epoch() {
        state.current_epoch = epoch;
    }

    (state, store)
}
```

## ConsensusWal (Write-Ahead Log)

The `ConsensusWal` provides crash recovery for the commit process. It uses a two-phase protocol:

1. **`log_commit_intent(target_height)`** — logged BEFORE starting block execution
2. **`log_commit_done(target_height)`** — logged AFTER persisting state; triggers WAL truncation

On startup, `check_recovery()` detects if a `CommitIntent` exists without a matching `CommitDone`, enabling the consensus engine to replay the interrupted commit.

```rust
use hotmint::storage::wal::{ConsensusWal, WalRecovery};

// Open or create WAL in data directory
let wal = ConsensusWal::open(&data_dir)?;

// Check for crash recovery at startup
match ConsensusWal::check_recovery(&data_dir)? {
    WalRecovery::Clean => { /* normal startup */ }
    // re-execute blocks from last_committed_height + 1 up to target_height
    WalRecovery::NeedsReplay { target_height } => { /* replay */ }
}

// Two-phase commit
wal.log_commit_intent(Height(42))?;
// ... execute block, persist state ...
wal.log_commit_done(Height(42))?;
```

A `NoopWal` implementation is provided for testing.

## PersistentEvidenceStore

The `PersistentEvidenceStore` persists equivocation proofs to vsdb, surviving node restarts:

```rust
use hotmint::storage::evidence_store::PersistentEvidenceStore;

let mut store = PersistentEvidenceStore::open(&data_dir)?;

// Store detected equivocation
store.put_evidence(proof);

// Get proofs not yet included in a block
let pending: Vec<EquivocationProof> = store.get_pending();

// Mark proofs as committed after block inclusion
store.mark_committed(view, validator_id);
```

Uses two vsdb collections internally:
- `proofs: MapxOrd<u64, EquivocationProof>` — keyed by auto-increment ID
- `committed: MapxOrd<u64, u8>` — committed-set keyed by the low 64 bits (little-endian) of `Blake3(view_le || validator_le)`; `mark_committed` also drops the proof from the `proofs` map

A `MemoryEvidenceStore` implementation is provided for testing.

## Data Directory Configuration

vsdb resolves its base directory from `$VSDB_BASE_DIR`, falling back to `$HOME/.vsdb` and finally to a process-private temporary directory — never the process working directory. Hotmint's persistent stores expect `vsdb_set_base_dir(<data_dir>)` to be called before `open()`, and keep their own metadata files in that same directory (`block_store.meta`, `consensus_state.meta`, `evidence_store.meta`, `consensus.wal`). There are two ways to point vsdb at a custom path:

### Environment Variable

```bash
export VSDB_BASE_DIR=/var/lib/hotmint/data
```

### Programmatic Configuration

```rust
// Must be called before any vsdb operation; can only be called once
vsdb::vsdb_set_base_dir("/var/lib/hotmint/data").unwrap();
```

`vsdb_set_base_dir()` accepts `impl AsRef<Path>` and returns an error if the database has already been initialized.

## Flush Semantics

By default vsdb writes are flushed asynchronously (the OS decides when to persist). Calling `vsdb_flush()` forces all pending writes to be synchronously flushed to disk.

Recommended flush points:
- After critical consensus state changes (view switches, QC updates, commits)
- After the application's `on_commit()` completes
- Before a graceful node shutdown

Both `VsdbBlockStore` and `PersistentConsensusState` expose a `.flush()` method that internally calls `vsdb::vsdb_flush()`.

## Advanced vsdb Features

Beyond basic KV storage, vsdb v16.x offers several advanced features that may be useful for future Hotmint extensions:

### VerMap — Versioned Storage

`VerMap` provides Git-style versioned storage with support for branching, committing, three-way merging, and rollback.

```rust
use vsdb::versioned::map::VerMap;

let mut m: VerMap<u32, String> = VerMap::new();
let main = m.main_branch();

m.insert(main, &1, &"hello".into())?;
m.commit(main)?;

let feat = m.create_branch("feature", main)?;
m.insert(feat, &1, &"updated".into())?;
m.commit(feat)?;

// Branches isolate changes
assert_eq!(m.get(main, &1)?, Some("hello".into()));
assert_eq!(m.get(feat, &1)?, Some("updated".into()));

// Three-way merge
m.merge(feat, main)?;
```

Potential use case: optimistic execution and rollback of application state.

### MptCalc / SmtCalc — Merkle Proofs

`MptCalc` (Merkle Patricia Trie) and `SmtCalc` (Sparse Merkle Tree) provide stateless Merkle root computation and proof generation.

`VerMapWithProof` combines versioned storage with Merkle root computation: `merkle_root(branch)` returns the 32-byte state root, updating an ephemeral trie incrementally from its last synced commit rather than recomputing the whole tree.

Potential use cases:
- Light client state verification
- Cross-chain state proofs
- Application-layer state commitments

## Implementing a Custom BlockStore

To use a different storage backend (e.g., SQLite, sled, or a remote database):

```rust
use hotmint::prelude::*;
use hotmint::consensus::store::BlockStore;
use hotmint::crypto::compute_block_hash;

struct SqliteBlockStore {
    conn: rusqlite::Connection,
}

impl SqliteBlockStore {
    fn new(path: &str) -> Self {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS blocks (
                hash BLOB PRIMARY KEY,
                height INTEGER NOT NULL,
                data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_height ON blocks(height);"
        ).unwrap();
        Self { conn }
    }
}

impl BlockStore for SqliteBlockStore {
    fn put_block(&mut self, block: Block) {
        let hash = compute_block_hash(&block);
        let data = postcard::to_allocvec(&block).unwrap();
        self.conn.execute(
            "INSERT OR REPLACE INTO blocks (hash, height, data) VALUES (?1, ?2, ?3)",
            (&hash.0[..], block.height.as_u64() as i64, &data),
        ).unwrap();
    }

    fn get_block(&self, hash: &BlockHash) -> Option<Block> {
        self.conn
            .query_row(
                "SELECT data FROM blocks WHERE hash = ?1",
                [&hash.0[..]],
                |row| {
                    let data: Vec<u8> = row.get(0)?;
                    Ok(postcard::from_bytes(&data).unwrap())
                },
            )
            .ok()
    }

    fn get_block_by_height(&self, h: Height) -> Option<Block> {
        self.conn
            .query_row(
                "SELECT data FROM blocks WHERE height = ?1",
                [h.as_u64() as i64],
                |row| {
                    let data: Vec<u8> = row.get(0)?;
                    Ok(postcard::from_bytes(&data).unwrap())
                },
            )
            .ok()
    }
}
```
