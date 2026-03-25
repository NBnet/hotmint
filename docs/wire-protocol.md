# Hotmint Wire Protocol Reference

This document defines the wire-level encoding standards that **all** hotmint
node implementations must follow, regardless of programming language or P2P
transport library.

## 1. Codec Framing (Consensus & Sync Messages)

Every consensus message and sync message on the wire uses a 1-byte tag prefix:

```
[0x00][raw postcard payload]     — uncompressed
[0x01][zstd-compressed postcard] — zstd level 3
```

### Encoding Rules

| Condition | Action |
|-----------|--------|
| Postcard payload <= 256 bytes | Prefix with `0x00`, send raw |
| Postcard payload > 256 bytes | Compress with zstd (level 3), prefix with `0x01` |

### Decoding Rules

1. Read the first byte (tag).
2. If `0x00`: the remainder is raw postcard — decode directly.
3. If `0x01`: the remainder is zstd-compressed — decompress first, then postcard-decode.
4. Any other tag: reject the message.

### Rationale

- Small messages (Vote, Prepare, Wish ~ 100-200 bytes) are sent uncompressed —
  the zstd framing overhead would exceed the savings.
- Large messages (Propose with full Block, SyncResponse with multiple blocks)
  benefit significantly from compression.
- The tag byte makes the format self-describing — any implementation can detect
  whether decompression is needed without out-of-band negotiation.
- Compression is part of the **hotmint protocol**, not the P2P transport layer,
  ensuring interoperability across different P2P libraries.

### Scope

| Protocol | Uses codec framing |
|----------|--------------------|
| `/hotmint/consensus/notif/1` | Yes |
| `/hotmint/consensus/reqresp/1` | Yes |
| `/hotmint/sync/1` | Yes |
| `/hotmint/pex/1` | No (raw postcard, small peer-exchange messages) |

## 2. Postcard Serialization

All structured data is serialized using [postcard](https://crates.io/crates/postcard),
a compact `#[no_std]`-compatible serde format using variable-length integer encoding.

### Enum Encoding

Rust enums are encoded with a varint discriminant followed by the variant's fields
(postcard's default serde representation).

### Newtype Wrappers

Types like `Height(u64)`, `ViewNumber(u64)`, `ValidatorId(u64)` are
transparent — they serialize as the inner value directly (varint-encoded).

### Fixed-Size Arrays

`BlockHash([u8; 32])` is serialized as 32 raw bytes (no length prefix).

`Vec<u8>` fields (e.g., `payload`, `Signature.0`, `PublicKey.0`) are
serialized with a varint length prefix followed by raw bytes.

## 3. ABCI IPC Protocol

The ABCI (Application Binary Consensus Interface) uses a **separate**
serialization format for cross-language interoperability:

- **Transport**: Unix domain socket
- **Framing**: 4-byte little-endian `u32` length prefix + protobuf payload
- **Serialization**: Protocol Buffers (see `proto/abci.proto`)
- **Max frame size**: 64 MB

The ABCI protocol is defined in `proto/abci.proto` and is the canonical
schema for Go (and other language) SDK implementations.

### Request/Response Flow

```
Engine (Rust) -> [4-byte LE len][protobuf Request]  -> Application (Go/Rust)
Engine (Rust) <- [4-byte LE len][protobuf Response] <- Application (Go/Rust)
```

## 4. Block Hash Computation

```
block_hash = Blake3(height_le64 || parent_hash_32 || view_le64 || proposer_le64 || app_hash_32 || payload)
```

All fields are serialized in little-endian byte order. The `hash` field
itself is excluded from the computation to avoid circularity.

`app_hash` is the application state root after executing the **parent** block
(delayed inclusion, following the CometBFT model).

## 5. Version History

| Version | Changes |
|---------|---------|
