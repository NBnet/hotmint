# hotmint-evm-execution

EVM block executor for the Hotmint EVM chain, powered by [revm](https://crates.io/crates/revm).

Implements the `Application` trait from `hotmint-consensus`, bridging the Hotmint consensus engine with full EVM execution.

## Components

| Type | Description |
|:-----|:------------|
| `EvmExecutor` | Main executor — owns EVM state, tx pool, and receipt storage |
| `SharedExecutor` | `Arc<EvmExecutor>` wrapper implementing `Application` for shared ownership |
| `EvmMempoolAdapter` | Implements `MempoolAdapter` trait, wrapping `EvmTxPool` for framework integration |

## Application Trait Implementation

| Method | Behavior |
|:-------|:---------|
| `validate_tx` | Decode + recover sender, check chain_id/nonce/balance/gas, return priority (effective tip) |
| `create_payload` | Collect pending txs from `EvmTxPool` (up to 4MB, block gas limit) |
| `execute_block` | Execute all txs via revm, accumulate receipts, compute state root |
| `on_commit` | Update block height, call `txpool.on_commit()` for nonce-based cleanup |
| `query` | Serve eth_getBalance, eth_getTransactionCount, eth_getCode, eth_getStorageAt |

## setup_nonce_fn

After placing `EvmExecutor` in an `Arc`, call `setup_nonce_fn()` to wire the tx pool's nonce lookup to the committed state:

```rust
let executor = Arc::new(EvmExecutor::from_genesis(&genesis));
executor.setup_nonce_fn(); // connects txpool nonce queries to state
```

Without this, `collect_payload` defaults all sender nonces to 0.

## License

GPL-3.0-only
