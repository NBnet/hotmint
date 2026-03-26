# hotmint-evm-node

Production EVM node binary and cluster management for the Hotmint EVM chain.

## Binaries

| Binary | Description |
|:-------|:------------|
| `hotmint-evm` | Full EVM validator node with P2P networking, consensus, and Ethereum JSON-RPC |
| `bench-evm` | EVM throughput benchmark — measures confirmed-on-chain TPS |

## Usage

```bash
# Run a node
hotmint-evm --home /path/to/node/home [--rpc-addr 127.0.0.1:8545]

# Run the benchmark
cargo run --release -p hotmint-evm-node --bin bench-evm
```

The node reads standard Hotmint config files (`config.toml`, `genesis.json`, `priv_validator_key.json`, `node_key.json`) plus an EVM-specific `evm-genesis.json` from the `config/` directory.

## Cluster Module

The `cluster` module (`src/cluster.rs`) provides EVM-specific cluster management helpers, wrapping the generic `hotmint-mgmt` framework:

```rust
use hotmint_evm_node::cluster::{init_evm_cluster, start_evm_nodes};

// Initialize: framework init + evm-genesis.json + eth RPC port allocation
let (state, eth_rpc_ports) = init_evm_cluster(
    &base_dir, 4, "my-chain", &evm_genesis, "127.0.0.1"
).unwrap();

// Start nodes with --rpc-addr for Ethereum JSON-RPC
let children = start_evm_nodes(&binary, &state, &base_dir, &eth_rpc_ports);
```

Used by the E2E tests and the benchmark.

## License

GPL-3.0-only
