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

Two exceptions matter to an implementer:

- Both notification protocols exchange a **32-byte chain-ID hash** as the substream handshake before any framed payload. Peers whose handshake does not match are rejected, which is how chain isolation is enforced.
- The consensus request-response protocol acknowledges a request with an **empty (0-byte) response frame** — no tag byte, no payload.

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
| `/hotmint/mempool/notif/1` | No (raw transaction bytes, not postcard) |
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
block_hash = Blake3(
      height_le64
   || parent_hash[32]
   || view_le64
   || proposer_le64
   || timestamp_le64                 # ms since the Unix epoch
   || app_hash[32]
   || evidence_count_le64
   || evidence[0] .. evidence[evidence_count - 1]
   || payload_len_le64 || payload
)

evidence[i] = validator_le64 || view_le64 || vote_type[1]
           || epoch_le64
           || block_hash_a[32] || sig_a_len_le64 || sig_a
           || ext_a_present[1]  [ || ext_a_len_le64 || ext_a ]
           || block_hash_b[32] || sig_b_len_le64 || sig_b
           || ext_b_present[1]  [ || ext_b_len_le64 || ext_b ]
```

`vote_type` is `0x00` for a `Vote` and `0x01` for a `Vote2`. Every
variable-length item (payload, signatures, vote extensions) carries an
8-byte little-endian length prefix; fixed-size items do not.

All integer fields are little-endian. The `hash` field itself is excluded
from the computation to avoid circularity.

`app_hash` is the application state root after executing the **parent** block
(delayed inclusion, following the CometBFT model).

## 5. Version History

| Version | Changes |
|---------|---------|
| 1 | Current revision, as described above: 1-byte tag codec (`0x00` raw postcard / `0x01` zstd level 3), postcard payloads, 32-byte chain-ID notification handshake, empty-frame acknowledgement on the consensus request-response protocol. |

There is no in-band version negotiation: the protocol revision is carried by
the `/1` suffix in each protocol path, and all five paths currently use `/1`.
