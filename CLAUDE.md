# Hotmint — Claude Code Project Guide

## What is this project?

Hotmint is a production-ready BFT consensus engine implementing the **HotStuff-2 two-chain commit protocol**. It combines Tendermint's ABCI ergonomics with HotStuff's optimal latency, in a pure-Rust implementation with no C/C++ dependencies in the consensus state machine, crypto, or storage stack (the P2P codec links zstd for payload compression).

## Workspace Layout

```
crates/
├── hotmint              # Library facade + node binary
├── hotmint-types        # Core data types (Block, Vote, QC, DC, TC, Epoch)
├── hotmint-consensus    # HotStuff-2 state machine (largest: ~6.4K LOC in src, ~7.7K incl. tests/)
├── hotmint-crypto       # Ed25519 + Blake3 signing/verification
├── hotmint-storage      # vsdb persistence layer
├── hotmint-network      # litep2p P2P networking (5 sub-protocols)
├── hotmint-mempool      # Priority tx pool with RBF
├── hotmint-api          # JSON-RPC + WebSocket (axum)
├── hotmint-abci         # IPC proxy for out-of-process apps
├── hotmint-abci-proto   # Protobuf definitions
├── hotmint-light        # Light client verification
├── hotmint-staking      # Validator lifecycle + slashing
└── hotmint-mgmt         # Cluster management tooling
```

## Build & Test

```bash
make all          # fmt + lint + build + test + doc
make lint         # cargo clippy --workspace --all-targets -- -D warnings
make test         # cargo test --workspace
make bench        # cargo bench --workspace
make demo         # 4-node multi-process demo
```

System dependency: `protobuf-compiler` (for proto code generation)

## Architecture

| Subsystem | Key files | Purpose |
|-----------|-----------|---------|
| View Protocol | `consensus/src/view_protocol.rs` | Enter → Propose → Vote → Prepare → Vote2 → Commit |
| State Machine | `consensus/src/state.rs` | Current view, locked QC, epoch tracking |
| Vote Collector | `consensus/src/vote_collector.rs` | Quorum aggregation (>2/3 of voting power), QC/TC formation |
| Commit Logic | `consensus/src/commit.rs` | Double Certificate triggers ancestor chain commit |
| Pacemaker | `consensus/src/pacemaker.rs` | Timeout scheduling, exponential backoff (1.5x, cap 30s) |
| Sync Layer | `consensus/src/sync.rs` | State sync + block catch-up |
| Storage | `storage/src/block_store.rs`, `storage/src/consensus_state.rs`, `storage/src/wal.rs` | VsdbBlockStore, PersistentConsensusState, ConsensusWal |
| Network | `network/src/service.rs` | litep2p multi-protocol event loop |
| Mempool | `mempool/src/lib.rs` | BTreeSet priority pool with RBF + eviction |
| API | `api/src/rpc.rs`, `api/src/http_rpc.rs` | JSON-RPC (TCP dispatch in rpc.rs; HTTP + WebSocket via axum in http_rpc.rs) |
| ABCI | `abci/src/` | Unix socket + protobuf framing |
| Light Client | `light/src/lib.rs` | Header verification (per-signer QC checks), MPT and SMT state proofs |
| Staking | `staking/src/` | Validator registration, delegation, slashing |
| Crypto | `crypto/src/lib.rs` | Ed25519 domain-separated signing, Blake3 hashing |

## Commands

- `/x-review` — deep regression analysis (supports: N commits, `all`, hash, range)
- `/x-fix` — fix audit backlog: resolve `docs/audit.md` → self-review → commit
- `/x-commit` — self-reviewing commit: review uncommitted changes → fix → commit
- `/x-overhaul` — full codebase overhaul: review all → fix → commit

Supporting documentation in `.claude/docs/`:
- `technical-patterns.md` — cataloged bug patterns for BFT consensus
- `review-core.md` — systematic review methodology
- `false-positive-guide.md` — rules for filtering spurious findings
- `patterns/` — per-subsystem review guides (consensus, network, storage, mempool, crypto, api)

## Conventions

- All clippy warnings are errors (CI enforced: `-D warnings`)
- **No `#[allow(...)]`** — fix warnings at the source, never suppress them
- **Prefer imports over inline paths** — avoid `std::foo::Bar::new()` inline in function bodies when the same path appears 3+ times in a file; add `use std::foo;` at file top (or `use std::foo::Bar;`) instead. Function-body `use` statements (scoped imports) are fine and don't count as inline paths. 1-2 inline uses of common `std::` items are acceptable.
- **Grouped imports** — merge common prefixes: `use std::sync::{Arc, Mutex};`
- **Doc-code alignment** — public API changes must update corresponding docs
- `parking_lot` for RwLock/Mutex (non-poisoning, fast uncontended)
- `tokio` async runtime (full features)
- `ruc` for error handling
- `postcard` for compact binary serialization
- Only 3 unsafe constructs (two `libc::kill` call sites plus the `unsafe extern "C"` declaration of `kill`), all in `crates/hotmint-mgmt/src/local.rs`; none on the consensus path
- Pluggable trait design: `Application`, `BlockStore`, `NetworkSink`, `Signer`, `Verifier`
