# hotmint-mempool

[![crates.io](https://img.shields.io/crates/v/hotmint-mempool.svg)](https://crates.io/crates/hotmint-mempool)
[![docs.rs](https://docs.rs/hotmint-mempool/badge.svg)](https://docs.rs/hotmint-mempool)

Transaction mempool for the [Hotmint](https://github.com/rust-util-collections/hotmint) BFT consensus framework.

A thread-safe, async transaction pool with **priority-based ordering**, Blake3-based deduplication, replace-by-fee (RBF), gas-aware selection, and configurable size limits. Provides a pluggable `MempoolAdapter` trait so chains can supply their own pool implementation.

## Features

- **Priority ordering** — transactions are proposed highest-priority first
- **Replace-by-fee (RBF)** — resubmitting the same tx with higher priority replaces the existing entry
- **Gas accounting** — `collect_payload_with_gas(max_bytes, max_gas)` respects both byte and gas limits
- **Eviction** — when the pool is full, a new tx with higher priority evicts the lowest-priority entry
- **Deduplication** — duplicate transactions (by Blake3 hash) are silently rejected
- **Post-commit recheck** — `recheck()` re-validates all pending txs against updated state
- **Pluggable** — `MempoolAdapter` trait lets any chain supply its own pool (e.g. EVM sender/nonce pool)

## MempoolAdapter Trait

```rust
#[async_trait]
pub trait MempoolAdapter: Send + Sync {
    async fn add_tx(&self, tx: Vec<u8>, priority: u64, gas_wanted: u64) -> bool;
    async fn size(&self) -> usize;
    async fn recheck(&self, validator: Box<dyn Fn(&[u8]) -> Option<(u64, u64)> + Send + Sync>);
}
```

The built-in `Mempool` struct implements this trait. EVM chains use `EvmMempoolAdapter` (in [`nbnet-execution`](https://github.com/rust-util-collections/nbnet)) which wraps `EvmTxPool` with sender/nonce semantics.

## Usage

```rust
use hotmint_mempool::Mempool;

let mempool = Mempool::new(10_000, 1_048_576); // max 10k txs, 1MB per tx

// Add a transaction with priority and gas
let accepted = mempool.add_tx_with_gas(b"tx-data".to_vec(), 100, 21_000).await;

// Collect payload for block proposal (up to 4MB, 30M gas)
let payload = mempool.collect_payload_with_gas(4_194_304, 30_000_000).await;

// Decode payload from a committed block
let txs: Vec<Vec<u8>> = Mempool::decode_payload(&payload);
```

### Integration with Application trait

```rust
use std::sync::Arc;
use hotmint_consensus::application::Application;
use hotmint_mempool::Mempool;

struct MyApp {
    mempool: Arc<Mempool>,
}

impl Application for MyApp {
    fn create_payload(&self, _ctx: &hotmint_types::context::BlockContext) -> Vec<u8> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.mempool.collect_payload_with_gas(4_194_304, 30_000_000))
    }
}
```

## License

GPL-3.0-only
