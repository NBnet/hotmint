# hotmint-abci

[![crates.io](https://img.shields.io/crates/v/hotmint-abci.svg)](https://crates.io/crates/hotmint-abci)
[![docs.rs](https://docs.rs/hotmint-abci/badge.svg)](https://docs.rs/hotmint-abci)

IPC proxy layer (Application Binary Consensus Interface) for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

Enables running the application logic in a separate process from the consensus engine, communicating over Unix domain sockets with length-prefixed protobuf frames.

## Architecture

```
┌──────────────┐  Unix socket  ┌──────────────────┐
│  Consensus   │◄─────────────►│   Application    │
│   Engine     │  protobuf msg │    Process       │
│              │               │                  │
│ IpcApp       │               │ IpcApp           │
│  Client      │               │  Server          │
└──────────────┘               └──────────────────┘
```

## Components

| Type | Description |
|:-----|:------------|
| `IpcApplicationClient` | Implements `Application` trait, forwards calls over IPC |
| `IpcApplicationServer` | Unix socket listener, dispatches requests to handler |
| `ApplicationHandler` | Callback trait for the application process |
| `Request` / `Response` | Protocol message types (protobuf-serialized) |

## Protocol

Requests and responses are exchanged as length-prefixed protobuf frames over a Unix domain socket:

```
[4 bytes: payload length (LE)] [payload: protobuf-encoded Request/Response]
```

Supported operations: `Info`, `InitChain`, `CreatePayload`, `ValidateBlock`, `ValidateTx`, `ExecuteBlock`, `OnCommit`, `OnEvidence`, `OnOfflineValidators`, `ExtendVote`, `VerifyVoteExtension`, `Query`, `ListSnapshots`, `LoadSnapshotChunk`, `OfferSnapshot`, `ApplySnapshotChunk`, `TracksAppHash`.

## Usage

### Application Process (Server)

```rust
use hotmint_abci::{ApplicationHandler, IpcApplicationServer};
use hotmint_types::context::OwnedBlockContext;

struct MyApp;

impl ApplicationHandler for MyApp {
    fn create_payload(&self, ctx: OwnedBlockContext) -> Vec<u8> {
        // ctx carries the block height and view for this payload
        vec![] // your payload logic
    }
    // ... implement other callbacks
}

let server = IpcApplicationServer::new("/tmp/myapp.sock", MyApp);
server.run().await.unwrap();
```

### Consensus Process (Client)

```rust
use hotmint_abci::IpcApplicationClient;

let app = IpcApplicationClient::new("/tmp/myapp.sock");
// Pass to ConsensusEngine as Box<dyn Application>
```

## License

GPL-3.0-only

### Decode errors

`protocol::decode_request`, `protocol::decode_response`, and the protobuf
`TryFrom` conversions return `hotmint_abci_proto::DecodeError`. Its `Protobuf`
variant wraps malformed wire data; `InvalidMessage` reports missing or invalid
ABCI fields. This replaces the previous `prost::DecodeError` return type.
