# hotmint-evm-types

Ethereum transaction types, genesis configuration, and chain config for the Hotmint EVM chain.

## Components

| Module | Description |
|:-------|:------------|
| `tx` | EIP-1559/Legacy transaction decoding, ECDSA recovery, validation (nonce, balance, gas, chain_id) |
| `genesis` | `EvmGenesis` — chain_id, alloc (address -> balance/nonce/code/storage), gas_limit, base_fee |
| `receipt` | `EvmReceipt` — execution status, gas_used, logs per transaction |
| `config` | `EvmChainConfig` — runtime chain parameters |

## Key Types

```rust
pub struct VerifiedTx {
    pub raw: Vec<u8>,           // RLP-encoded bytes
    pub envelope: TxEnvelope,   // decoded transaction
    pub sender: Address,        // recovered sender
    pub tx_hash: B256,          // transaction hash
}

pub struct EvmGenesis {
    pub chain_id: u64,
    pub alloc: BTreeMap<Address, GenesisAlloc>,
    pub gas_limit: u64,
    pub base_fee_per_gas: u64,
    pub coinbase: Address,
    pub timestamp: u64,
}
```

## License

GPL-3.0-only
