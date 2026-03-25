# Architecture

Hotmint follows a modular, layered architecture inspired by Tendermint. Each concern — consensus logic, cryptography, networking, storage, and application — lives in its own crate with clear trait boundaries.

## Workspace Layout

```
hotmint/
├── Cargo.toml                     # workspace root
├── crates/
│   ├── hotmint-types/             # core data types (minimal dependencies)
│   ├── hotmint-crypto/            # cryptography implementations
│   ├── hotmint-consensus/         # consensus state machine
│   ├── hotmint-storage/           # persistent storage, WAL, evidence store (vsdb)
│   ├── hotmint-network/           # P2P networking (litep2p)
│   ├── hotmint-mempool/           # priority mempool with RBF and gas accounting
│   ├── hotmint-abci/              # IPC proxy for out-of-process apps
│   ├── hotmint-api/               # HTTP/WebSocket/TCP JSON-RPC API
│   ├── hotmint-staking/           # staking: validator registration, delegation, slashing, rewards
│   ├── hotmint-light/             # light client: header verification, validator set tracking
│   └── hotmint/                   # top-level library facade
└── docs/
```

## Dependency Graph

```
hotmint (library facade — re-exports everything)
  ├── hotmint-consensus ──> hotmint-types
  ├── hotmint-crypto    ──> hotmint-types
  ├── hotmint-storage   ──> hotmint-consensus, vsdb
  ├── hotmint-network   ──> hotmint-consensus, litep2p
  ├── hotmint-abci      ──> hotmint-consensus, hotmint-types
  ├── hotmint-staking   ──> hotmint-types
  ├── hotmint-light     ──> hotmint-types
  ├── hotmint-mempool   (standalone, no consensus/network/storage deps)
  └── hotmint-api       ──> hotmint-mempool
```

Key design rule: **the consensus engine has no dependency on any concrete networking or storage crate**. It communicates with the outside world exclusively through trait objects (`Box<dyn BlockStore>`, `Box<dyn NetworkSink>`, `Box<dyn Application>`, `Box<dyn Signer>`), connected via `tokio::mpsc` channels.

## Crate Responsibilities

### hotmint-types

The foundational crate with minimal dependencies. Defines all data types shared across the system:

- `Block`, `BlockHash`, `Height` — chain primitives
- `ViewNumber` — consensus view tracking
- `Vote`, `VoteType` — voting messages
- `QuorumCertificate`, `DoubleCertificate`, `TimeoutCertificate` — aggregate proofs
- `ConsensusMessage` — the wire protocol enum
- `ValidatorId`, `ValidatorInfo`, `ValidatorSet` — validator management
- `Signature`, `PublicKey`, `AggregateSignature` — cryptographic primitives
- `Signer`, `Verifier` — abstract cryptographic traits
- `Epoch`, `EpochNumber` — epoch management

### hotmint-crypto

Concrete cryptographic implementations:

- `Ed25519Signer` — implements the `Signer` trait using ed25519-dalek
- `Ed25519Verifier` — implements the `Verifier` trait
- `compute_block_hash()` — Blake3 block hashing

### hotmint-consensus

The core consensus state machine, entirely independent of I/O:

- `ConsensusEngine` — the main event loop (`tokio::select!`)
- `ConsensusState` — mutable consensus state (current view, locks, role)
- `view_protocol` — steady-state view protocol (Paper Figure 1)
- `pacemaker` — timeout and view change (Paper Figure 2)
- `vote_collector` — vote aggregation and QC formation
- `commit` — two-chain commit rule
- `leader` — round-robin leader election
- `metrics` — Prometheus metrics collection

Also defines the pluggable trait interfaces:
- `BlockStore` — block persistence
- `NetworkSink` — message transport
- `Application` — ABCI-like application lifecycle

Each trait includes an in-memory/no-op stub implementation for development use. The `hotmint-abci` crate provides `IpcApplicationClient`, which implements `Application` by forwarding calls over a Unix domain socket to an out-of-process application.

### hotmint-abci

IPC proxy layer (Application Binary Consensus Interface) for running applications as separate processes:

- `IpcApplicationClient` — implements `Application` by forwarding calls over a Unix domain socket using length-prefixed protobuf frames
- `IpcApplicationServer` + `ApplicationHandler` — listens on a Unix socket, dispatches requests to user-provided handler
- `OwnedBlockContext` — owned version of `BlockContext` for cross-process serialization

This enables applications written in any language to participate in consensus, communicating via a simple length-prefixed protobuf protocol over Unix stream sockets.

### hotmint-storage

Production-grade persistent storage backed by vsdb:

- `VsdbBlockStore` — implements `BlockStore` with `MapxOrd` for by-hash and by-height indexing
- `PersistentConsensusState` — persists critical consensus state (view, locks, committed height) for crash recovery

### hotmint-network

Real P2P networking using litep2p:

- `NetworkService` — manages litep2p connections, protocol handlers, and event routing
- `Litep2pNetworkSink` — implements `NetworkSink` for production use
- `PeerMap` — bidirectional `ValidatorId ↔ PeerId` mapping

Uses four sub-protocols:
- `/hotmint/consensus/notif/1` — notification protocol for broadcast
- `/hotmint/consensus/reqresp/1` — request-response protocol for directed messages
- `/hotmint/sync/1` — request-response protocol for block synchronization
- `/hotmint/pex/1` — peer exchange protocol for peer discovery

### hotmint-mempool

Priority-based transaction pool:

- Priority ordering (highest first) with Replace-by-Fee (RBF) support
- Gas-aware payload collection (`collect_payload_with_gas`)
- Post-commit re-validation (`recheck()`) to evict stale transactions
- Deduplication via Blake3 transaction hashing
- Configurable size limits (transaction count and byte size)
- Length-prefixed payload encoding for block inclusion

### hotmint-api

HTTP/WebSocket/TCP JSON-RPC server:

- `HttpRpcServer` — axum-based HTTP `POST /` + WebSocket `GET /ws`
- `RpcServer` — TCP-based JSON-RPC server (newline-delimited)
- Event subscription via WebSocket with `SubscribeFilter` (event types, height range, tx hash)
- Methods: `status`, `submit_tx`, `get_block`, `get_block_by_hash`, `get_header`, `get_commit_qc`, `get_tx`, `get_block_results`, `get_validators`, `get_epoch`, `get_peers`, `query`, `verify_header`
- Per-IP rate limiting on `submit_tx`

### hotmint-staking

Staking toolkit for DPoS economic models:

- Validator registration with minimum self-stake, delegation, undelegation with time-locked unbonding
- Slashing for double-signing (`SlashReason::DoubleSign`) and downtime (`SlashReason::Downtime`)
- Automatic jailing with configurable duration, unjailing after jail period
- Reputation scoring, block rewards, validator set computation (`compute_validator_updates`)
- Pluggable `StakingStore` trait for any backend

### hotmint-light

Light client verification:

- `LightClient` struct with `verify_header` (QC signature verification) and `update_validator_set`
- MPT state proof verification via vsdb `MptProof`

## Core Trait Abstractions

The four pluggable traits define the boundary between the consensus engine and the outside world:

```rust
// Cryptographic signing — swap implementations without touching consensus
trait Signer: Send + Sync {
    fn sign(&self, message: &[u8]) -> Signature;
    fn public_key(&self) -> PublicKey;
    fn validator_id(&self) -> ValidatorId;
}

// Block persistence — in-memory for tests, vsdb for production
trait BlockStore: Send + Sync {
    fn put_block(&mut self, block: Block);
    fn get_block(&self, hash: &BlockHash) -> Option<Block>;
    fn get_block_by_height(&self, h: Height) -> Option<Block>;
    fn put_commit_qc(&mut self, height: Height, qc: QuorumCertificate);
    fn get_commit_qc(&self, height: Height) -> Option<QuorumCertificate>;
    fn put_tx_index(&mut self, tx_hash: [u8; 32], height: Height, index: u32);
    fn get_tx_location(&self, tx_hash: &[u8; 32]) -> Option<(Height, u32)>;
    fn put_block_results(&mut self, height: Height, results: EndBlockResponse);
    fn get_block_results(&self, height: Height) -> Option<EndBlockResponse>;
    fn get_blocks_in_range(&self, from: Height, to: Height) -> Vec<Block>;
    fn tip_height(&self) -> Height;
    fn flush(&self);
}

// Network transport — channels for testing, litep2p for production
trait NetworkSink: Send + Sync {
    fn broadcast(&self, msg: ConsensusMessage);
    fn send_to(&self, target: ValidatorId, msg: ConsensusMessage);
}

// Application lifecycle — your business logic
// All methods have default no-op implementations.
trait Application: Send + Sync {
    // Startup & genesis
    fn info(&self) -> AppInfo;                              // last_block_height + last_block_app_hash
    fn init_chain(&self, app_state: &[u8]) -> Result<BlockHash>;
    // Block lifecycle
    fn create_payload(&self, ctx: &BlockContext) -> Vec<u8>;
    fn validate_block(&self, block: &Block, ctx: &BlockContext) -> bool;
    fn validate_tx(&self, tx: &[u8], ctx: Option<&TxContext>) -> TxValidationResult;
    fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse>;
    fn on_commit(&self, block: &Block, ctx: &BlockContext) -> Result<()>;
    // Evidence & liveness
    fn on_evidence(&self, proof: &EquivocationProof) -> Result<()>;
    fn on_offline_validators(&self, offline: &[OfflineEvidence]) -> Result<()>;
    // Vote extensions (ABCI++)
    fn extend_vote(&self, block: &Block, ctx: &BlockContext) -> Option<Vec<u8>>;
    fn verify_vote_extension(&self, ext: &[u8], block_hash: &BlockHash, validator: ValidatorId) -> bool;
    // State sync snapshots
    fn list_snapshots(&self) -> Vec<SnapshotInfo>;
    fn load_snapshot_chunk(&self, height: Height, chunk_index: u32) -> Vec<u8>;
    fn offer_snapshot(&self, snapshot: &SnapshotInfo) -> SnapshotOfferResult;
    fn apply_snapshot_chunk(&self, chunk: Vec<u8>, chunk_index: u32) -> ChunkApplyResult;
    // Queries & config
    fn query(&self, path: &str, data: &[u8]) -> Result<QueryResponse>;
    fn tracks_app_hash(&self) -> bool;
}
```

## Message Flow

```
Client ──tx──> Mempool ──payload──> Application.create_payload()
                                        │
Leader: Propose ───broadcast───> Replicas
                                     │
Replicas: VoteMsg ───send_to───> Leader
                                     │
Leader: Prepare (QC) ──broadcast──> Replicas
                                     │
Replicas: Vote2Msg ──send_to──> Next Leader
                                     │
Next Leader: DoubleCert formed ──> commit chain
                                     │
                              Application.on_commit()
```

## Design Decisions

### Why trait objects instead of generics?

Trait objects (`Box<dyn T>`) are used for `BlockStore`, `NetworkSink`, `Application`, and `Signer` rather than generic type parameters. This choice:

- Keeps the `ConsensusEngine` type signature simple
- Allows runtime composition (e.g., switching between in-memory and persistent storage based on config)
- Avoids monomorphization bloat for what are typically single-instance objects

### Why tokio::mpsc for engine ↔ network communication?

The consensus engine receives messages through a `tokio::mpsc::Receiver` rather than directly calling network APIs. This decouples the consensus logic from network implementation details and makes the engine trivially testable with in-memory channels.

### Why owned values in BlockStore?

`BlockStore` returns `Option<Block>` (owned) rather than references. This is a deliberate choice for vsdb compatibility — vsdb stores data on disk and cannot return references to in-memory data. The owned-value pattern works uniformly across both in-memory and persistent implementations.
