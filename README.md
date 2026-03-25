# Hotmint

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/rust-util-collections/hotmint/actions/workflows/ci.yml/badge.svg)](https://github.com/rust-util-collections/hotmint/actions/workflows/ci.yml)
[![HotStuff-2](https://img.shields.io/badge/protocol-HotStuff--2-purple.svg)](https://arxiv.org/abs/2301.03253)
[![crates.io](https://img.shields.io/crates/v/hotmint.svg)](https://crates.io/crates/hotmint)

**A next-generation BFT consensus framework** — built from scratch in Rust, combining Tendermint's battle-tested architecture with HotStuff-2's optimal two-chain commit protocol.

Every critical layer — from the consensus state machine down to the on-disk LSM-Tree storage engine — is independently developed, giving the project **complete sovereignty over its entire technology stack**.

---

## Origins & Motivation

### The Industry Baseline

[CometBFT](https://github.com/cometbft/cometbft) (formerly Tendermint) has served as the backbone of the Cosmos ecosystem for years, proving the viability of BFT consensus for production blockchains. Its clean separation of consensus and application (ABCI) set the gold standard for developer ergonomics. However, years of evolution have surfaced fundamental limitations:

- **Three-phase voting** (Propose → Pre-vote → Pre-commit) imposes an inherent extra round-trip on every block
- **Go runtime** — garbage collection causes tail-latency jitter that is structurally impossible to eliminate in a latency-sensitive consensus protocol
- **RocksDB dependency** — the storage layer relies on a massive C++ codebase, introducing cross-language build complexity, C memory safety risks, and limited control over the most critical data path
- **Accumulated technical debt** — organic growth over many years makes deep architectural changes increasingly costly

### The Breakthrough

In 2023, Dahlia Malkhi and Ling Ren published [HotStuff-2](https://arxiv.org/abs/2301.03253), proving that **two-chain commit is sufficient for optimal BFT consensus** — achieving the same safety guarantees as Tendermint's three phases while eliminating an entire voting round. Confirmation latency drops, and view-change mechanics simplify dramatically: from complex Nil-vote collection to a linear Wish → TimeoutCert aggregation.

### The Thesis

Hotmint was born from the convergence of three insights:

1. **HotStuff-2's two-chain commit** eliminates Tendermint's latency overhead while preserving its proven safety properties (f < n/3 Byzantine tolerance)
2. **Rust's zero-cost abstractions** deliver C-level performance with compile-time memory safety — no GC pauses, no data races, no use-after-free
3. **A fully self-developed storage stack** eliminates the RocksDB/LevelDB dependency entirely — pure Rust from the application API down to the LSM-Tree write path

The result: a consensus framework where **every performance-critical path is written in safe Rust**, and the team has complete control from the consensus commit decision down to the on-disk byte layout.

---

## Vision

Hotmint is not just a consensus engine. It is the foundation for a **next-generation full-stack blockchain framework**.

### Phase 1 — Production-Ready AppChain Engine *(current)*

A battle-hardened BFT consensus engine that any Rust developer can embed to build application-specific blockchains, with the same ABCI-style ergonomics that made Tendermint successful — but with lower latency, stronger type safety, and zero C/C++ dependencies in the critical path.

### Phase 2 — EVM-Compatible Chain (Hotmint-EVM)

Build a production-grade EVM-compatible chain by combining the best-in-class component for each layer:

- **[revm](https://github.com/bluealloy/revm)** — the world's fastest EVM execution engine (adopted by Paradigm, OP Stack, Arbitrum)
- **[alloy](https://github.com/alloy-rs)** — modern Ethereum primitives, RLP codec, and Web3 RPC types
- **AI-ported [Substrate Pallets](https://github.com/niccolocorsini/polkadot-sdk/tree/master/substrate/frame)** — battle-tested economic models (staking, governance, multi-asset) ported into Hotmint's `std + vsdb + serde` environment

This "hybrid architecture" assembles each ecosystem's strongest module, sidestepping any single ecosystem's historical baggage.

### Phase 3 — Full-Stack Blockchain Framework

The long-term goal is a **"hexagonal warrior" (六边形战士)** — a framework that excels across every dimension of blockchain infrastructure:

| Dimension | Advantage |
|:----------|:----------|
| **Consensus** | HotStuff-2 — lower latency than Tendermint, simpler than PBFT |
| **Execution** | revm — world's fastest EVM engine |
| **Storage** | vsdb + mmdb — pure-Rust, zero C deps, Git-model versioning + Merkle proofs |
| **Networking** | litep2p — lightweight, from Polkadot ecosystem |
| **Business Logic** | AI-ported Substrate Pallets — type-safe Rust, audited by top security firms |
| **Developer Experience** | ABCI-style trait API, Go SDK, cross-language IPC, cluster management tooling |

> 📖 **[Full roadmap and security audit →](docs/security-audit-and-roadmap.md)**

---

## Full-Stack Self-Developed Core

Unlike frameworks that aggregate third-party C/C++ components for their most critical paths, Hotmint's deepest layers are **independently developed** under the same organization ([rust-util-collections](https://github.com/rust-util-collections)):

### 🔷 Consensus — Hotmint *(this project)*

HotStuff-2 two-chain commit protocol, implemented from scratch. The consensus state machine has **zero I/O dependencies** — all storage, networking, and application logic is injected through four pluggable traits. Domain-separated signing (`chain_id_hash + epoch + view + block_hash`) prevents all cross-chain, cross-epoch, and cross-message replay attacks.

### 🔷 Storage — [vsdb](https://github.com/rust-util-collections/vsdb) + [mmdb](https://github.com/rust-util-collections/mmdb)

**[vsdb](https://crates.io/crates/vsdb)** (Version-controlled Storage Database) is a high-performance embedded key-value database with a standard-collections API:

- `Mapx` / `MapxOrd` — persistent `HashMap` / `BTreeMap` replacements
- `VerMap` — **Git-model versioning**: branching, commits, three-way merge, rollback, garbage collection over a COW B+ tree with structural sharing
- `MptCalc` / `SmtCalc` — stateless **Merkle Patricia Trie** and **Sparse Merkle Tree** computation layers
- `VerMapWithProof` — integrates `VerMap` with `MptCalc` for versioned 32-byte Merkle root commitments

**[mmdb](https://github.com/rust-util-collections/mmdb)** is the storage engine underneath vsdb — a **pure-Rust LSM-Tree** that replaces RocksDB/LevelDB:

- WAL with group commit and crash recovery
- SST files with prefix compression, bloom filters, leveled compaction
- MVCC snapshots, block cache (moka LRU), multi-threaded background compaction
- Performance comparable to RocksDB in typical workloads; 250+ tests

This gives Hotmint **100% control** over the entire data path — from the consensus commit decision down to the on-disk compaction strategy — with **zero C/C++ dependencies**.

### 🔷 Error Handling — [ruc](https://github.com/rust-util-collections/ruc)

Chained error tracing library, also independently developed. Provides rich error context propagation throughout the entire stack.

---

## Protocol

Hotmint implements the HotStuff-2 two-chain commit protocol ([arXiv:2301.03253](https://arxiv.org/abs/2301.03253)):

```
Block  ←──  QC (2f+1 votes)  ←──  Double Cert (2f+1 votes on QC)  ──→  Commit
```

Each view follows a 5-step protocol:

```
Enter  →  Propose  →  Vote  →  Prepare (QC)  →  Vote2  →  [DC triggers Commit]
```

- **Safety**: Locking rule (`justify.rank ≥ locked_qc.rank`) prevents conflicting commits; double certificate commits the block and all uncommitted ancestors
- **Liveness**: Timeout → Wish → TimeoutCert mechanism with exponential backoff (1.5×, capped at 30s)
- **Epochs**: Validator set changes take effect at `commit_view + 2`, ensuring all honest nodes agree on the transition point

📖 **[Full protocol specification →](docs/protocol.md)**

---

## Architecture

### Workspace

| Crate | Description |
|:------|:------------|
| [hotmint](https://crates.io/crates/hotmint) | Library facade — re-exports all crates; includes `hotmint-node` binary |
| [hotmint-types](https://crates.io/crates/hotmint-types) | Core data types: Block, QC, DC, TC, Vote, ValidatorSet, Epoch |
| [hotmint-crypto](https://crates.io/crates/hotmint-crypto) | Ed25519 signing + batch verification, Blake3 hashing |
| [hotmint-consensus](https://crates.io/crates/hotmint-consensus) | Consensus state machine: engine, pacemaker, vote collector, sync |
| [hotmint-storage](https://crates.io/crates/hotmint-storage) | Persistent storage backends (vsdb) |
| [hotmint-network](https://crates.io/crates/hotmint-network) | P2P networking (litep2p): 4 sub-protocols (consensus, reqresp, sync, PEX) |
| [hotmint-mempool](https://crates.io/crates/hotmint-mempool) | Priority mempool with RBF, gas-aware selection, deduplication |
| [hotmint-api](https://crates.io/crates/hotmint-api) | HTTP/WebSocket JSON-RPC + TCP JSON-RPC server |
| [hotmint-abci](https://crates.io/crates/hotmint-abci) | IPC proxy for out-of-process apps (Unix socket + protobuf) |
| [hotmint-staking](https://crates.io/crates/hotmint-staking) | Staking toolkit: validator registration, delegation, slashing, rewards |
| [hotmint-light](https://crates.io/crates/hotmint-light) | Light client: header verification and validator set tracking |

### Pluggable Traits

The consensus engine is fully decoupled from all I/O through four pluggable traits:

| Trait | Purpose | Built-in Implementations |
|:------|:--------|:-------------------------|
| `Application` | ABCI-like app lifecycle | `NoopApplication`, `IpcApplicationClient` |
| `BlockStore` | Block persistence | `MemoryBlockStore`, `VsdbBlockStore` |
| `NetworkSink` | Message transport | `ChannelNetwork`, `Litep2pNetworkSink` |
| `Signer` | Cryptographic signing | `Ed25519Signer` |

📖 **[Architecture →](docs/architecture.md)** · **[Core types →](docs/types.md)** · **[Wire protocol →](docs/wire-protocol.md)**

---

## Quick Start

```bash
# build and test
cargo build --workspace && cargo test --workspace

# run the 4-node in-process demo
cargo run --bin hotmint-demo

# or initialize and run a production node
cargo run --bin hotmint-node -- init
cargo run --bin hotmint-node -- node
```

📖 **[Getting started guide →](docs/getting-started.md)**

---

## Examples

| Example | Description | Run |
|:--------|:------------|:----|
| [demo](examples/demo) | Minimal 4-node in-process cluster with a counting app | `cargo run --bin hotmint-demo` |
| [evm-chain](examples/evm-chain) | Complete EVM chain using **revm** + vsdb MPT state trie | `cargo run --bin hotmint-evm-chain` |
| [utxo-chain](examples/utxo-chain) | Bitcoin-style UTXO chain with ed25519 sigs + SMT proofs | `cargo run --bin hotmint-utxo-chain` |
| [cluster-node](examples/cluster-node) | Production-style P2P node with persistent storage, sync, PEX | `cargo run --bin hotmint-cluster-node` |
| [bench-consensus](examples/bench-consensus) | Raw consensus throughput benchmark | `make bench-consensus` |
| [bench-ipc](examples/bench-ipc) | ABCI IPC overhead benchmark (Unix socket + protobuf) | `make bench-ipc` |

---

## SDK & Tools

| Component | Description |
|:----------|:------------|
| [Go SDK](sdk/go) | Out-of-process application framework for Go — `Application` interface + Unix socket IPC server |
| [hotmint-mgmt](tools/hotmint-mgmt) | Cluster management CLI: `init` / `start` / `stop` / `deploy` / `logs` (local + remote SSH) |

---

## Usage

Add `hotmint` as a dependency:

```toml
[dependencies]
hotmint = { git = "https://github.com/rust-util-collections/hotmint" }
tokio = { version = "1", features = ["full"] }
ruc = "9.3"
```

Implement the `Application` trait — all methods have default no-op implementations:

```rust
use ruc::*;
use hotmint::prelude::*;
use hotmint::consensus::application::Application;

struct MyApp;

impl Application for MyApp {
    fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
        println!("height {} — {} txs", ctx.height.as_u64(), txs.len());
        Ok(EndBlockResponse::default())
    }

    fn on_commit(&self, block: &Block, _ctx: &BlockContext) -> Result<()> {
        println!("committed height {}", block.height.as_u64());
        Ok(())
    }
}
```

Build a cluster and run:

```rust
// see examples/demo for the complete working code
let engine = ConsensusEngine::new(state, store, network, app, signer, rx, config);
tokio::spawn(async move { engine.run().await });
```

Three deployment modes — all interoperable in the same cluster:

| Mode | Application Language | Communication |
|:-----|:--------------------|:--------------|
| **Embedded** | Rust (same process) | Direct trait calls |
| **Go ABCI** | Go | Unix socket + protobuf |
| **Rust ABCI** | Rust (separate process) | Unix socket + protobuf |

📖 **[Application trait guide →](docs/application.md)** · **[Storage guide →](docs/storage.md)** · **[Networking guide →](docs/networking.md)** · **[Mempool & API →](docs/mempool-api.md)**

---

## Technology Stack

| Component | Implementation | Origin |
|:----------|:---------------|:-------|
| Consensus Protocol | HotStuff-2 (arXiv:2301.03253) | Self-developed |
| Storage Engine | [vsdb](https://crates.io/crates/vsdb) + [mmdb](https://crates.io/crates/mmdb) (pure-Rust LSM-Tree) | Self-developed |
| Error Handling | [ruc](https://crates.io/crates/ruc) | Self-developed |
| Signatures | Ed25519 ([ed25519-dalek](https://crates.io/crates/ed25519-dalek)) | Community |
| Hashing | [Blake3](https://crates.io/crates/blake3) | Community |
| Networking | [litep2p](https://crates.io/crates/litep2p) (Polkadot ecosystem) | Community |
| Async Runtime | [Tokio](https://crates.io/crates/tokio) | Community |
| Serialization | [serde](https://crates.io/crates/serde) + [postcard](https://crates.io/crates/postcard) / [Protobuf](https://crates.io/crates/prost) | Community |
| Metrics | [prometheus-client](https://crates.io/crates/prometheus-client) | Community |

---

## Documentation

| Guide | Description |
|:------|:------------|
| [Getting Started](docs/getting-started.md) | Installation, quick start, first integration |
| [Protocol](docs/protocol.md) | HotStuff-2 two-chain commit, view protocol, pacemaker |
| [Architecture](docs/architecture.md) | Module structure, dependency graph, design decisions |
| [Application](docs/application.md) | `Application` trait — ABCI-like lifecycle, epoch transitions, evidence |
| [Consensus Engine](docs/consensus-engine.md) | Engine internals: state machine, event loop, vote collection |
| [Core Types](docs/types.md) | Block, QC, DC, TC, Vote, ValidatorSet, signing bytes, wire format |
| [Cryptography](docs/crypto.md) | Signer/Verifier traits, Ed25519, aggregate signatures, custom signers |
| [Storage](docs/storage.md) | BlockStore trait, vsdb persistence, crash recovery, Merkle proofs |
| [Networking](docs/networking.md) | NetworkSink trait, litep2p P2P, PEX, block sync, dynamic peers |
| [Mempool & API](docs/mempool-api.md) | Priority mempool, JSON-RPC (TCP + HTTP + WebSocket) |
| [Metrics](docs/metrics.md) | Prometheus metrics, health interpretation, Grafana queries |
| [Wire Protocol](docs/wire-protocol.md) | Codec framing, postcard format, ABCI IPC protocol, block hash spec |
| [Security Audit & Roadmap](docs/security-audit-and-roadmap.md) | CometBFT gap analysis, security audit, evolution roadmap |

---

## References

| Paper | Link |
|:------|:-----|
| HotStuff-2: Optimal Two-Chain BFT (2023) | [arXiv:2301.03253](https://arxiv.org/abs/2301.03253) |
| HotStuff: BFT Consensus (PODC 2019) | [arXiv:1803.05069](https://arxiv.org/abs/1803.05069) |
| Tendermint: Latest Gossip on BFT (2018) | [arXiv:1807.04938](https://arxiv.org/abs/1807.04938) |

## License

GPL-3.0
