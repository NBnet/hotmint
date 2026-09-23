# utxo-chain

Bitcoin-style UTXO chain example for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

A complete working example of a UTXO chain with ed25519 signatures, persistent state via vsdb (`VerMapWithProof` + SMT proofs), and address-indexed queries via `SlotDex`.

## Binaries

| Binary | Description |
|:-------|:------------|
| `utxo-chain-example` | 4-validator demo: Alice sends 1 COIN to Bob each block (30s) |
| `bench-utxo` | Throughput benchmark: 10 UTXO transfers/block |

## Run

```bash
# Demo (30 seconds)
cargo run -p utxo-chain-example

# Benchmark
cargo run --release -p utxo-chain-example --bin bench-utxo
```

## Architecture

- `utxo_types.rs` — `OutPoint`, `TxInput`, `TxOutput`, `UtxoTx` with CBOR encoding and blake3 hashing
- `utxo_state.rs` — `UtxoState`: persistent UTXO set (SMT), address index (`SlotDex128`), total supply
- `utxo_app.rs` — `UtxoApplication`: full `Application` trait with transaction validation, execution, and query
- `app.rs` — `DemoUtxoApp`: wrapper that auto-generates signed Alice→Bob transfers
- `bench.rs` — `UtxoBenchApp`: standalone benchmark application

## Features

- Full transaction validation: double-spend, ownership, signature, amount conservation
- Sparse Merkle Tree proofs via `VerMapWithProof<[u8; 36], TxOutput, SmtCalc>`
- Address-indexed UTXO queries with pagination
- Ed25519 `verify_strict` for signature verification

The `prove` query takes a 36-byte outpoint key and returns the queried key hash
(32 bytes), a leaf-present flag (1 byte), and, when present, the terminal leaf's
key hash (32 bytes), value length (little-endian u32), and value bytes. The
remaining bytes are sibling hashes (32 bytes each, root first). A terminal leaf
with a different key hash proves non-membership; its key and value must be
preserved when reconstructing a vsdb `SmtProof` for verification.

## License

GPL-3.0-only
