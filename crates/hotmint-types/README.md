# hotmint-types

[![crates.io](https://img.shields.io/crates/v/hotmint-types.svg)](https://crates.io/crates/hotmint-types)
[![docs.rs](https://docs.rs/hotmint-types/badge.svg)](https://docs.rs/hotmint-types)

Core data types for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

This crate defines all shared primitives used across the Hotmint ecosystem with minimal dependencies (only `serde` and `ruc`). It is the foundation that every other Hotmint crate depends on.

## Types

| Type | Description |
|:-----|:------------|
| `Block`, `BlockHash`, `Height` | Chain primitives — block structure, 32-byte Blake3 hash, block height |
| `ViewNumber` | Monotonically increasing consensus view number |
| `Vote`, `VoteType` | Phase-1 and phase-2 voting messages |
| `QuorumCertificate` | Aggregate proof from 2f+1 validators on a block |
| `DoubleCertificate` | QC-of-QC that triggers commit (two-chain rule) |
| `TimeoutCertificate` | Aggregated timeout proof for view change |
| `ConsensusMessage` | Wire protocol enum (Propose, Vote, Prepare, Wish, TC, StatusCert, Evidence) |
| `ValidatorId`, `ValidatorInfo`, `ValidatorSet` | Validator identity, metadata, and set management |
| `Signature`, `PublicKey`, `AggregateSignature` | Cryptographic primitives |
| `Signer`, `Verifier` | Abstract traits for pluggable signature schemes |
| `Epoch`, `EpochNumber` | Epoch management for validator set transitions |

### ConsensusMessage::Propose

The `Propose` variant carries everything a replica needs to validate and commit:

```rust
Propose {
    block: Box<Block>,                           // the proposed block
    justify: Box<QuorumCertificate>,             // QC certifying the parent
    double_cert: Option<Box<DoubleCertificate>>, // optional fast-forward commit
    signature: Signature,                         // proposer's Ed25519 signature
    ancestor_blocks: Vec<Block>,                 // uncommitted ancestors for chain recovery
}
```

The `ancestor_blocks` field ensures replicas who missed earlier proposals (e.g., skipped a view via TC) can reconstruct the full commit chain.

## License

GPL-3.0-only
