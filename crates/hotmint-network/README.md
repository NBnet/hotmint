# hotmint-network

[![crates.io](https://img.shields.io/crates/v/hotmint-network.svg)](https://crates.io/crates/hotmint-network)
[![docs.rs](https://docs.rs/hotmint-network/badge.svg)](https://docs.rs/hotmint-network)

P2P networking layer for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

Implements the `NetworkSink` trait from `hotmint-consensus` using [litep2p](https://crates.io/crates/litep2p) for real multi-process / multi-machine deployments. Messages are serialized with CBOR, with optional zstd compression for large frames.

## Sub-Protocols

| Protocol | Path | Use |
|:---------|:-----|:----|
| Consensus Notification | `/hotmint/consensus/notif/1` | `broadcast()` — fire-and-forget to all peers |
| Consensus Request-Response | `/hotmint/consensus/reqresp/1` | `send_to()` — directed message to a specific peer |
| Sync Request-Response | `/hotmint/sync/reqresp/1` | Block sync protocol |
| Mempool Notification | `/hotmint/mempool/notif/1` | `broadcast_tx()` — transaction gossip |
| PEX Notification | `/hotmint/pex/notif/1` | Peer exchange for discovery |

## NetworkSink Trait Methods

| Method | Description |
|:-------|:------------|
| `broadcast(msg)` | Broadcast consensus message to all peers |
| `send_to(target, msg)` | Send directed message to specific validator |
| `broadcast_tx(tx_bytes)` | Gossip raw transaction to all peers |
| `broadcast_evidence(proof)` | Broadcast equivocation evidence |
| `on_epoch_change(epoch, vs)` | Notify network of validator set change |

## Components

| Component | Description |
|:----------|:------------|
| `NetworkService` | Manages litep2p connections and event routing |
| `Litep2pNetworkSink` | Implements `NetworkSink` for production use |
| `PeerMap` | Bidirectional `ValidatorId <-> PeerId` mapping |
| `PeerBook` | Persistent peer discovery state |

## License

GPL-3.0-only
