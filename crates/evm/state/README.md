# hotmint-evm-state

EVM world state backed by [vsdb](https://crates.io/crates/vsdb) for the Hotmint EVM chain.

Manages account balances, nonces, contract code, and storage slots. Provides a `CacheDB` adapter for [revm](https://crates.io/crates/revm) execution and computes Merkle Patricia Trie state roots for block headers.

## Features

- **Persistent state** — all account data stored in vsdb (pure-Rust LSM-Tree)
- **revm integration** — `CacheDB` adapter bridges vsdb to revm's `Database` trait
- **State root** — deterministic MPT root computation after each block
- **Genesis initialization** — `EvmState::from_genesis()` populates initial accounts from `EvmGenesis`

## Key API

```rust
pub struct EvmState {
    pub config: EvmChainConfig,
    // ...
}

impl EvmState {
    pub fn from_genesis(genesis: &EvmGenesis) -> Self;
    pub fn get_balance(&self, addr: &Address) -> U256;
    pub fn get_nonce(&self, addr: &Address) -> u64;
    pub fn get_code(&self, addr: &Address) -> Vec<u8>;
    pub fn get_storage(&self, addr: &Address, slot: &U256) -> U256;
}
```

## License

GPL-3.0-only
