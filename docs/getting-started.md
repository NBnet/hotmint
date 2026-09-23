# Getting Started

## Prerequisites

- **Rust** 1.88 or newer on a stable toolchain (edition 2024). CI builds with `dtolnay/rust-toolchain@stable`; no `rust-version` is pinned, so treat 1.88 as a floor rather than a guarantee.
- **protobuf-compiler** — required at build time by the proto code generation used by `hotmint-abci-proto` and litep2p (e.g. `apt-get install protobuf-compiler`).

## Installation

Add `hotmint` as a dependency:

```toml
[dependencies]
hotmint = { git = "https://github.com/NBnet/hotmint" }
tokio = { version = "1", features = ["full"] }
ruc = "11.0.1"
```

## Quick Start

```bash
# clone the repository
git clone https://github.com/NBnet/hotmint.git
cd hotmint

# build all crates
cargo build --workspace

# run all tests
cargo test --workspace

# initialize a local node and run it. `init` scaffolds a single-validator
# genesis plus a fullnode config.toml; to run a validator against an ABCI app,
# set mode = "validator" and proxy_app = "unix:///path/to/app.sock" first.
cargo run --bin hotmint-node -- init
cargo run --bin hotmint-node -- node
```

## Minimal Integration (Embedded Application)

The simplest way to use hotmint is to implement the `Application` trait and embed it directly into your node binary. All methods have default no-op implementations, so you only need to implement the ones your application cares about.

### Step 1: Define Your Application

```rust
use ruc::*;
use hotmint::prelude::*;
use hotmint::consensus::application::Application;

struct MyApp;

impl Application for MyApp {
    fn on_commit(&self, block: &Block, _ctx: &BlockContext) -> Result<()> {
        println!("committed block at height {}", block.height.as_u64());
        Ok(())
    }
}
```

### Step 2: Set Up a Node

The recommended way to run a node is using the same configuration files as `hotmint-node`: `config.toml`, `genesis.json`, `priv_validator_key.json`, and `node_key.json`. See `examples/cluster-node/` for a complete working example of an embedded node with real P2P networking.

Key steps:

1. Load config, keys, and genesis from disk
2. Create `NetworkService` with litep2p for P2P connectivity
3. Create `ConsensusEngine` with your `Application` implementation
4. Spawn network + consensus tasks

```rust
// Load configuration (same files as hotmint-node)
let config = NodeConfig::load(&config_dir.join("config.toml"))?;
let priv_key = PrivValidatorKey::load(&config_dir.join("priv_validator_key.json"))?;
let node_key = NodeKey::load(&config_dir.join("node_key.json"))?;
let genesis = GenesisDoc::load(&config_dir.join("genesis.json"))?;

// Create P2P network
let (peer_map, known_addresses) =
    config::parse_persistent_peers(&config.p2p.persistent_peers, &genesis)?;
let handles = NetworkService::create(NetworkConfig {
    listen_addr,                 // Multiaddr parsed from config.p2p.laddr
    peer_map,
    known_addresses,
    keypair: Some(litep2p_keypair),   // derived from node_key.json
    peer_book,                   // Arc<tokio::sync::RwLock<PeerBook>>
    pex_config: config.pex.clone(),
    relay_consensus: config.node.relay_consensus,
    initial_validators,          // Vec<(ValidatorId, PublicKey)>
    chain_id_hash: state.chain_id_hash,
})?;

// Create and run consensus engine with your embedded application
let engine = ConsensusEngine::new(
    state,
    store,
    Box::new(handles.sink),
    Box::new(MyApp),
    Box::new(signer),
    handles.msg_rx,
    EngineConfig { ... },
);

tokio::spawn(async move { handles.service.run().await });
engine.run().await;
```

## Deployment Modes

| Mode | Binary | When to Use |
|------|--------|-------------|
| **Embedded (single-process)** | Your own binary | Rust applications, maximum performance |
| **ABCI dual-process (Go)** | `hotmint-node` + Go app | Go applications via `sdk/go/` |
| **ABCI dual-process (Rust)** | `hotmint-node` + Rust ABCI server | Rust apps needing process isolation |

All three modes are interoperable — a cluster can mix different deployment modes and even different operating systems (macOS, Linux, FreeBSD).

## CLI Flags

The `hotmint-node` binary takes six subcommands:

| Subcommand | Description |
|:-----------|:------------|
| `init` | Scaffold `<home>` with `config.toml`, `genesis.json`, and keys |
| `node` | Run the consensus node (flags below) |
| `gen-validator-key` | Generate an Ed25519 validator key (`-o <FILE>`, `--validator-id <N>`) |
| `gen-node-key` | Generate an Ed25519 P2P identity key (`-o <FILE>`) |
| `show-validator` | Print validator info from a key file (`-f <FILE>`) |
| `show-node-id` | Print the node's `PeerId` from a key file (`-f <FILE>`) |

`--home <PATH>` is a global flag and may appear before or after the subcommand. The `node` subcommand accepts the following flags:

| Flag | Description | Default |
|:-----|:------------|:--------|
| `--home <PATH>` | Set the home directory for config and data | `~/.hotmint` |
| `node --proxy-app <ADDR>` | Unix socket address of the ABCI application | `""` (empty; no-op app used when not set) |
| `node --p2p-laddr <ADDR>` | P2P listen address (multiaddr format) | `/ip4/0.0.0.0/tcp/20000` |
| `node --rpc-laddr <ADDR>` | JSON-RPC listen address | `127.0.0.1:20001` |

Examples:

```bash
# initialize with a custom home directory
cargo run --bin hotmint-node -- --home /data/mynode init

# run a node with custom addresses
cargo run --bin hotmint-node -- --home /data/mynode node \
    --proxy-app unix:///tmp/myapp.sock \
    --p2p-laddr /ip4/0.0.0.0/tcp/20000 \
    --rpc-laddr 0.0.0.0:20001
```

## Next Steps

- [Application](application.md) — full lifecycle: `execute_block`, `on_commit`, `query`
- [Storage](storage.md) — swap in persistent vsdb storage for production
- [Networking](networking.md) — P2P networking with litep2p, peer exchange, block sync
- [Mempool & API](mempool-api.md) — accept external transactions via JSON-RPC
- [Wire Protocol](wire-protocol.md) — wire format reference for node implementors
