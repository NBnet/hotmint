# hotmint-consensus

[![crates.io](https://img.shields.io/crates/v/hotmint-consensus.svg)](https://crates.io/crates/hotmint-consensus)
[![docs.rs](https://docs.rs/hotmint-consensus/badge.svg)](https://docs.rs/hotmint-consensus)

HotStuff-2 consensus state machine and engine for the [Hotmint](https://github.com/NBnet/hotmint) BFT framework.

This is the core crate of Hotmint. It implements the full HotStuff-2 protocol — two-chain commit, five-step view protocol, pacemaker with exponential backoff — and is completely decoupled from I/O through pluggable trait interfaces.

## Architecture

```
ConsensusEngine
  ├── ConsensusState      mutable state (view, locks, role, epoch)
  ├── view_protocol       steady-state protocol (Paper Figure 1)
  ├── pacemaker           timeout & view change (Paper Figure 2)
  ├── vote_collector      vote aggregation & QC formation
  ├── commit              two-chain commit rule
  ├── leader              round-robin / weighted leader election
  └── liveness            offline validator tracking
```

## Pluggable Traits

| Trait | Purpose | Built-in Stub |
|:------|:--------|:--------------|
| `Application` | ABCI-like app lifecycle | `NoopApplication` |
| `BlockStore` | Block persistence | `MemoryBlockStore` |
| `NetworkSink` | Message transport + tx gossip | `Litep2pNetworkSink` |

## Key Design Points

- **Ancestor blocks in proposals** — when a leader proposes, it includes all uncommitted ancestor blocks so replicas who missed earlier views can still commit the full chain
- **Cross-epoch tolerance** — the engine retains the previous epoch's validator set to verify in-flight messages (TCs, Wishes) formed before an epoch transition
- **Vote extensions** (ABCI++) — validators can attach application-specific data to Vote2 messages, delivered to the next proposer

## Usage

```rust
use hotmint_consensus::engine::ConsensusEngineBuilder;
use hotmint_consensus::state::ConsensusState;
use hotmint_consensus::store::MemoryBlockStore;
use hotmint_consensus::application::NoopApplication;

let engine = ConsensusEngineBuilder::new()
    .state(ConsensusState::new(vid, validator_set))
    .store(MemoryBlockStore::new_shared())
    .network(network_sink)           // Box<dyn NetworkSink>
    .app(Box::new(NoopApplication))
    .signer(Box::new(signer))
    .messages(msg_rx)
    .verifier(Box::new(verifier))
    .build()
    .unwrap();

tokio::spawn(async move { engine.run().await });
```

### Implement Application

All methods have default no-op implementations. Lifecycle: `execute_block(txs, ctx)` -> `on_commit(block, ctx)`.

```rust
use ruc::*;
use hotmint_types::Block;
use hotmint_consensus::application::Application;

struct MyApp;

impl Application for MyApp {
    fn on_commit(&self, block: &Block, _ctx: &hotmint_types::context::BlockContext) -> Result<()> {
        println!("committed height {}", block.height.as_u64());
        Ok(())
    }
}
```

## License

GPL-3.0-only
