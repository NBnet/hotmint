# hotmint-api

[![crates.io](https://img.shields.io/crates/v/hotmint-api.svg)](https://crates.io/crates/hotmint-api)
[![docs.rs](https://docs.rs/hotmint-api/badge.svg)](https://docs.rs/hotmint-api)

JSON-RPC API server for the [Hotmint](https://github.com/rust-util-collections/hotmint) BFT consensus framework.

Provides a TCP-based newline-delimited JSON-RPC server and an HTTP/WebSocket server for querying node status, submitting transactions, and subscribing to chain events.

## RPC Methods

| Method | Description | Response |
|:-------|:------------|:---------|
| `status` | Node status (view, height, epoch, mempool size) | `StatusInfo` |
| `submit_tx` | Submit hex-encoded transaction to mempool + gossip to peers | `TxResult { accepted }` |
| `get_block` | Query block by height | `BlockInfo` |
| `get_validators` | Current validator set | `Vec<ValidatorInfoResponse>` |
| `get_peers` | Connected peer status | `Vec<PeerStatus>` |
| `get_epoch` | Current epoch info | `EpochInfo` |

## Transaction Flow

When `submit_tx` is called:
1. Rate-limit check (per-IP, 100 tx/sec)
2. `Application::validate_tx()` — application-level validation
3. `MempoolAdapter::add_tx()` — add to the pluggable mempool
4. `NetworkSink::broadcast_tx()` — gossip to all connected peers

The mempool and network sink are trait objects, allowing any chain to plug in its own transaction pool and gossip mechanism.

## Client Examples

```bash
# query status
echo '{"method":"status","params":{},"id":1}' | nc 127.0.0.1 26657

# submit transaction (hex-encoded)
echo '{"method":"submit_tx","params":"deadbeef","id":2}' | nc 127.0.0.1 26657
```

## License

GPL-3.0-only
