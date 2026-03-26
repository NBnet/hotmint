# Hotmint Documentation

Comprehensive documentation for the Hotmint BFT consensus framework.

## Table of Contents

| Document | Description |
|:---------|:------------|
| [Getting Started](getting-started.md) | Installation, quick start, and first integration |
| [Protocol](protocol.md) | HotStuff-2 protocol specification: two-chain commit, view protocol, pacemaker |
| [Architecture](architecture.md) | Module structure, dependency graph, design decisions |
| [Application](application.md) | `Application` trait guide with ABCI-like lifecycle |
| [Consensus Engine](consensus-engine.md) | Engine internals: state machine, event loop, vote collection |
| [Core Types](types.md) | Type reference: blocks, certificates, votes, validators, signing bytes |
| [Cryptography](crypto.md) | `Signer`/`Verifier` traits, Ed25519, aggregate signatures |
| [Storage](storage.md) | `BlockStore` trait, vsdb persistence, crash recovery |
| [Networking](networking.md) | `NetworkSink` trait, litep2p P2P transport |
| [Mempool & API](mempool-api.md) | Transaction mempool and JSON-RPC server |
| [Metrics](metrics.md) | Prometheus metrics and observability |
| [Wire Protocol](wire-protocol.md) | Codec framing, postcard format, ABCI IPC protocol, block hash spec |
| [Security Audit & Roadmap](security-audit-and-roadmap.md) | Security audit, CometBFT feature gap analysis, and evolution roadmap |
