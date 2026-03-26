# hotmint-evm-txpool

Ethereum-compatible transaction pool for the Hotmint EVM chain.

Manages pending and queued transactions with (sender, nonce) ordering, EIP-1559 fee-based priority, and replacement-by-fee support.

## Features

- **Sender/nonce ordering** — transactions are grouped by sender, ordered by nonce for correct execution
- **Pending vs queued** — nonce-continuous txs are pending (ready for inclusion); gapped txs are queued
- **EIP-1559 tip priority** — transactions sorted by effective gas tip (max_fee - base_fee, capped at max_priority_fee)
- **Replacement-by-fee** — same (sender, nonce) with higher tip replaces existing tx
- **Nonce promotion** — when a gap-filling tx arrives, queued txs automatically promote to pending
- **Per-sender limits** — configurable max pending + queued per sender

## Key API

```rust
pub struct EvmTxPool { /* ... */ }

impl EvmTxPool {
    pub fn new(config: EvmTxPoolConfig) -> Self;
    pub fn set_nonce_fn(&self, f: Box<dyn Fn(&Address) -> u64 + Send + Sync>);
    pub fn submit_tx(&self, raw: &[u8]) -> Result<B256, String>;
    pub fn collect_payload(&self, max_bytes: usize, max_gas: u64) -> Vec<u8>;
    pub fn on_commit(&self, nonce_fn: &dyn Fn(&Address) -> u64);
    pub fn pending_count(&self) -> usize;
    pub fn queued_count(&self) -> usize;
}
```

## License

GPL-3.0-only
