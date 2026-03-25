# Mempool & JSON-RPC API

## Mempool

The `Mempool` is a thread-safe, priority-based transaction pool with deduplication, eviction, Replace-by-Fee (RBF), and gas-aware payload collection.

### Construction

```rust
use hotmint::mempool::Mempool;

// custom limits: max 10,000 transactions, max 1MB per transaction
let mempool = Mempool::new(10_000, 1_048_576);

// default limits: 10,000 txs, 1MB
let mempool = Mempool::default();
```

### Adding Transactions

```rust
use std::sync::Arc;

let mempool = Arc::new(Mempool::default());

// add_tx with priority (higher = included first)
let accepted = mempool.add_tx(b"transfer alice bob 100".to_vec(), 10).await;

// add_tx with priority and gas estimate
let accepted = mempool.add_tx_with_gas(b"transfer alice bob 100".to_vec(), 10, 21000).await;
```

Transactions are deduplicated by their Blake3 hash. Duplicate transactions are silently rejected. If the pool is full, new transactions with higher priority than the lowest-priority entry will evict it.

**Replace-by-Fee (RBF):** Resubmitting the same transaction bytes with a higher priority replaces the existing pending entry, allowing wallets to bump fees on stuck transactions.

### Collecting Payload for Block Proposal

When the leader needs to propose a block, it calls `collect_payload` from the `Application::create_payload` method:

```rust
use hotmint::consensus::application::Application;

struct MyApp {
    mempool: Arc<Mempool>,
}

impl Application for MyApp {
    fn create_payload(&self, _ctx: &BlockContext) -> Vec<u8> {
        let rt = tokio::runtime::Handle::current();
        // collect up to 1MB of transactions, respecting max_gas_per_block
        rt.block_on(self.mempool.collect_payload_with_gas(1_048_576, 30_000_000))
    }
}
```

`collect_payload` drains transactions in priority order (highest first) until the byte or gas limit is reached. Transactions that exceed the remaining gas budget are skipped (not dropped). Transactions are encoded in a length-prefixed format:

```
[u32_le: tx1_len][tx1_bytes][u32_le: tx2_len][tx2_bytes]...
```

### Decoding Payload

```rust
let txs: Vec<Vec<u8>> = Mempool::decode_payload(&block.payload);
for tx in &txs {
    // process each transaction
}
```

### Pool Status

```rust
let size = mempool.size().await;
println!("pending transactions: {}", size);
```

### Post-Commit Re-validation

After each block commit, call `recheck()` to re-validate pending transactions against the updated application state. Transactions that are no longer valid (e.g., nonce violations, insufficient balance) are evicted:

```rust
mempool.recheck(|tx_bytes| {
    // return None to evict, Some((priority, gas_wanted)) to keep
    match app.validate_tx(tx_bytes, Some(&ctx)) {
        r if r.valid => Some((r.priority, r.gas_wanted)),
        _ => None,
    }
}).await;
```

## JSON-RPC API

Hotmint provides two RPC server implementations:

- **`HttpRpcServer`** — axum-based HTTP `POST /` + WebSocket `GET /ws` (recommended for dApps)
- **`RpcServer`** — TCP-based newline-delimited JSON (legacy, lightweight)

Both expose the same method set. The HTTP server additionally supports WebSocket event subscriptions with filtering.

### HTTP/WebSocket Server

The `HttpRpcServer` uses axum and supports:
- `POST /` — standard JSON-RPC requests
- `GET /ws` — WebSocket connection for real-time event subscriptions

WebSocket clients receive `ChainEvent` messages (`NewBlock`, `TxCommitted`, `EpochChange`) and can send a `SubscribeFilter` to filter by event types, height range, or transaction hash.

### TCP Server

The `RpcServer` provides a TCP-based JSON-RPC interface for external clients to query node status and submit transactions.

### Setup

```rust
use std::sync::Arc;
use tokio::sync::watch;
use hotmint::mempool::Mempool;
use hotmint::api::rpc::{RpcServer, RpcState};

let mempool = Arc::new(Mempool::default());

// status channel: (current_view, last_committed_height, epoch, validator_count, epoch_start_view)
// update this from your Application::on_commit handler
let (status_tx, status_rx) = watch::channel((0u64, 0u64, 0u64, 4usize, 0u64));

use std::sync::RwLock;
use hotmint::consensus::engine::SharedBlockStore;
use hotmint::consensus::store::MemoryBlockStore;

let store: SharedBlockStore =
    Arc::new(RwLock::new(Box::new(MemoryBlockStore::new())));
let (_peer_tx, peer_info_rx) = watch::channel(vec![]);

let (_vs_tx, validator_set_rx): (watch::Sender<Vec<ValidatorInfoResponse>>, _) = watch::channel(vec![]);

let rpc_state = RpcState {
    validator_id: 0,
    mempool: mempool.clone(),
    status_rx,
    store,
    peer_info_rx,
    validator_set_rx,
};

let server = RpcServer::bind("127.0.0.1:20001", rpc_state).await.unwrap();
let addr = server.local_addr(); // actual bound address (useful if port was 0)
tokio::spawn(async move { server.run().await });
```

### Updating Status

Wire the status channel into your application's commit handler:

```rust
struct MyApp {
    mempool: Arc<Mempool>,
    status_tx: watch::Sender<(u64, u64, u64, usize, u64)>,
}

impl Application for MyApp {
    fn on_commit(&self, block: &Block, ctx: &BlockContext) -> ruc::Result<()> {
        let _ = self.status_tx.send((
            block.view.as_u64(),
            block.height.as_u64(),
            ctx.epoch.as_u64(),
            ctx.validator_set.validator_count(),
            0, // epoch_start_view
        ));
        Ok(())
    }

    fn create_payload(&self, _ctx: &BlockContext) -> Vec<u8> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.mempool.collect_payload(1_048_576))
    }
}
```

### Protocol

The RPC server uses newline-delimited JSON over TCP. Each request is a single JSON object terminated by `\n`, and each response is a single JSON object terminated by `\n`.

### Request Format

```json
{
    "method": "method_name",
    "params": { ... },
    "id": 1
}
```

### Response Format

Success:
```json
{
    "result": { ... },
    "error": null,
    "id": 1
}
```

Error:
```json
{
    "result": null,
    "error": { "code": -32601, "message": "method not found" },
    "id": 1
}
```

### Methods

#### `status`

Returns the current node status.

Request:
```bash
echo '{"method":"status","params":{},"id":1}' | nc 127.0.0.1 20001
```

Response:
```json
{
    "result": {
        "validator_id": 0,
        "current_view": 42,
        "last_committed_height": 15,
        "mempool_size": 3
    },
    "error": null,
    "id": 1
}
```

#### `submit_tx`

Submit a transaction (hex-encoded bytes).

Request:
```bash
echo '{"method":"submit_tx","params":"48656c6c6f","id":2}' | nc 127.0.0.1 20001
```

Response:
```json
{
    "result": { "accepted": true },
    "error": null,
    "id": 2
}
```

The transaction is hex-decoded and added to the mempool. `accepted: false` means the transaction was rejected (duplicate, pool full, or failed `Application::validate_tx`).

#### `get_block`

Returns a committed block by height.

Request:
```bash
echo '{"method":"get_block","params":{"height":5},"id":3}' | nc 127.0.0.1 20001
```

#### `get_block_by_hash`

Returns a committed block by its hash (hex-encoded).

Request:
```bash
echo '{"method":"get_block_by_hash","params":{"hash":"abcd1234..."},"id":4}' | nc 127.0.0.1 20001
```

#### `get_validators`

Returns the current validator set.

Request:
```bash
echo '{"method":"get_validators","params":{},"id":5}' | nc 127.0.0.1 20001
```

#### `get_epoch`

Returns the current epoch number and metadata.

Request:
```bash
echo '{"method":"get_epoch","params":{},"id":6}' | nc 127.0.0.1 20001
```

#### `get_peers`

Returns the list of connected peers and their status.

Request:
```bash
echo '{"method":"get_peers","params":{},"id":7}' | nc 127.0.0.1 20001
```

#### `get_header`

Returns a lightweight block header (without payload) by height.

Request:
```bash
echo '{"method":"get_header","params":{"height":5},"id":8}' | nc 127.0.0.1 20001
```

#### `get_commit_qc`

Returns the Quorum Certificate that committed a block at the given height.

Request:
```bash
echo '{"method":"get_commit_qc","params":{"height":5},"id":9}' | nc 127.0.0.1 20001
```

#### `get_tx`

Query a committed transaction by its Blake3 hash (hex-encoded). Returns the transaction data, height, and index within the block.

Request:
```bash
echo '{"method":"get_tx","params":{"hash":"abcd1234..."},"id":10}' | nc 127.0.0.1 20001
```

#### `get_block_results`

Returns the `EndBlockResponse` (events, app_hash, transaction hashes) for a committed block.

Request:
```bash
echo '{"method":"get_block_results","params":{"height":5},"id":11}' | nc 127.0.0.1 20001
```

#### `query`

Application-defined state query. Returns data with optional Merkle proof.

Request:
```bash
echo '{"method":"query","params":{"path":"balance","data":"616c696365"},"id":12}' | nc 127.0.0.1 20001
```

#### `verify_header`

Light client header verification — verifies QC signatures and validator set.

Request:
```bash
echo '{"method":"verify_header","params":{"height":5},"id":13}' | nc 127.0.0.1 20001
```

### Rate Limiting

The `submit_tx` method is rate-limited per IP address (default: 100 requests/second) using a token-bucket algorithm. Stale entries are pruned every 60 seconds.

### Types

```rust
pub struct RpcRequest {
    pub method: String,
    pub params: serde_json::Value,
    pub id: u64,
}

pub struct RpcResponse {
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
    pub id: u64,
}

pub struct RpcError {
    pub code: i32,
    pub message: String,
}

pub struct StatusInfo {
    pub validator_id: u64,
    pub current_view: u64,
    pub last_committed_height: u64,
    pub mempool_size: usize,
    pub epoch: u64,
    pub validator_count: usize,
}

pub struct TxResult {
    pub accepted: bool,
}
```

## Full Example: Node with Mempool and RPC

```rust
use std::sync::Arc;
use ruc::*;
use tokio::sync::watch;
use hotmint::prelude::*;
use hotmint::consensus::application::Application;
use hotmint::consensus::engine::ConsensusEngine;
use hotmint::consensus::state::ConsensusState;
use hotmint::consensus::store::MemoryBlockStore;
use hotmint::crypto::Ed25519Signer;
use hotmint::mempool::Mempool;
use hotmint::api::rpc::{RpcServer, RpcState};

struct TxCounterApp {
    mempool: Arc<Mempool>,
    status_tx: watch::Sender<(u64, u64, u64, usize, u64)>,
}

impl Application for TxCounterApp {
    fn create_payload(&self, _ctx: &BlockContext) -> Vec<u8> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.mempool.collect_payload(1_048_576))
    }

    fn validate_tx(&self, tx: &[u8], _ctx: Option<&TxContext>) -> TxValidationResult {
        if !tx.is_empty() && tx.len() <= 4096 {
            TxValidationResult::accept(0)
        } else {
            TxValidationResult::reject()
        }
    }

    fn on_commit(&self, block: &Block, ctx: &BlockContext) -> Result<()> {
        let txs = Mempool::decode_payload(&block.payload);
        let _ = self.status_tx.send((
            block.view.as_u64(),
            block.height.as_u64(),
            ctx.epoch.as_u64(),
            ctx.validator_set.validator_count(),
            0, // epoch_start_view
        ));
        println!(
            "height={} txs={} view={}",
            block.height.as_u64(),
            txs.len(),
            block.view.as_u64(),
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mempool = Arc::new(Mempool::default());
    let (status_tx, status_rx) = watch::channel((0u64, 0u64, 0u64, 4usize, 0u64));

    use std::sync::RwLock;
    use hotmint::consensus::engine::SharedBlockStore;
    use hotmint::consensus::store::MemoryBlockStore;

    let store: SharedBlockStore =
        Arc::new(RwLock::new(Box::new(MemoryBlockStore::new())));
    let (_peer_tx, peer_info_rx) = watch::channel(vec![]);
    let (_vs_tx, validator_set_rx): (watch::Sender<Vec<ValidatorInfoResponse>>, _) = watch::channel(vec![]);

    // start RPC server
    let rpc_state = RpcState {
        validator_id: 0,
        mempool: mempool.clone(),
        status_rx,
        store,
        peer_info_rx,
        validator_set_rx,
    };
    let server = RpcServer::bind("127.0.0.1:20001", rpc_state).await.unwrap();
    println!("RPC listening on {}", server.local_addr());
    tokio::spawn(async move { server.run().await });

    let app = TxCounterApp {
        mempool: mempool.clone(),
        status_tx,
    };

    // ... set up validators and consensus engine as shown in getting-started.md
    // ... pass `app` to ConsensusEngine::new()
}
```
