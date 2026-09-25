# utxo-chain

Bitcoin-style UTXO chain example for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

A complete working example of a UTXO chain with ed25519 signatures, persistent state via vsdb (`VerMapWithProof` + SMT proofs), and address-indexed queries via `SlotDex`.

## Binaries

| Binary | Description |
|:-------|:------------|
| `utxo-chain-example` | 4-validator cluster demo (30s): runs `cluster-node` with `NoopApplication`; the UTXO application is not yet wired into a node binary |
| `bench-utxo` | 10s block-production benchmark against 4 `cluster-node` processes (NoopApp); no UTXO execution |

## Run

```bash
# Demo (30 seconds)
cargo run -p utxo-chain-example

# Benchmark
cargo run --release -p utxo-chain-example --bin bench-utxo
```

## Architecture

- `utxo_types.rs` — `OutPoint`, `TxInput`, `TxOutput`, `UtxoTx` with postcard encoding and blake3 hashing
- `utxo_state.rs` — `UtxoState`: persistent UTXO set (SMT), address index (`SlotDex128`), total supply
- `utxo_app.rs` — `UtxoApplication`: full `Application` trait with transaction validation, execution, and query
- `app.rs` — `DemoUtxoApp`: wrapper that auto-generates signed Alice→Bob transfers
- `bench.rs` — the `bench-utxo` binary: 10s multi-process block-production benchmark

## Features

- Full transaction validation: double-spend, ownership, signature, amount conservation
- Sparse Merkle Tree proofs via `VerMapWithProof<[u8; 36], TxOutput, SmtCalc>`
- Address-indexed UTXO queries with pagination and streaming balance aggregation
- Ed25519 `verify_strict` for signature verification

The `prove` query takes a 36-byte outpoint key and returns
`SmtProof::to_bytes()` in `QueryResponse.data`. VSDB 17.0.7 provides this
versioned encoding, its matching `SmtProof::from_bytes()` decoder, and serde
support for both SMT and MPT proofs. Proof generation synchronizes the current
working state through `prove_at`; no preceding `utxo_root` query is required.

```rust
use hotmint_consensus::application::Application;
use vsdb::{SmtCalc, SmtProof};

let key = outpoint.to_key();
let response = app.query("prove", &key)?;
let proof = SmtProof::from_bytes(&response.data)?;
assert!(SmtCalc::verify_proof(&trusted_root, &key, &proof)?);
// After verification: Some(value) proves inclusion; None proves exclusion.
let output = proof.value();
```

`trusted_root` must be obtained independently for the same application state.
The lightweight client offers the equivalent
`LightClient::verify_smt_state_proof` helper. A valid absence proof may contain
a different terminal leaf; the VSDB codec preserves that leaf automatically.

**Client migration:** this replaces the example's previous hand-built proof
layout. Clients must switch to the VSDB decoder; old proof bytes are not
accepted by the new codec. The `prove` query still returns bytes in `data`,
and stored UTXOs and their root hashes are unchanged by this codec change.

## License

GPL-3.0-only
