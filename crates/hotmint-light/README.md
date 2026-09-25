# hotmint-light

[![crates.io](https://img.shields.io/crates/v/hotmint-light.svg)](https://crates.io/crates/hotmint-light)
[![docs.rs](https://docs.rs/hotmint-light/badge.svg)](https://docs.rs/hotmint-light)

Light client verification library for the [Hotmint](https://github.com/NBnet/hotmint) BFT consensus framework.

Checks block headers against quorum certificates (QCs) using a trusted validator set and checkpoint. Validator set transitions must be supplied as trusted checkpoints by the caller.

## Features

- **Header verification** — verify that a QC was signed by more than 2/3 of the known validator set's voting power
- **Validator set tracking** — replace the validator set at an externally trusted checkpoint
- **Hash chain tracking** — require each header to extend the last accepted checkpoint
- **State proofs** — verify MPT and SMT inclusion/exclusion proofs with VSDB's versioned proof codec

Header fields are not independently authenticated: the header omits the payload and evidence needed to recompute the certified block hash. Supply headers from a trusted source before relying on their fields, including `app_hash` for state proofs.

## Usage

```rust
use hotmint_light::LightClient;
use hotmint_crypto::Ed25519Verifier;

let mut lc = LightClient::new_with_trusted_hash(
    trusted_validator_set,
    trusted_height,
    trusted_hash,
    chain_id_hash,
);

// Verify a block header + QC
lc.verify_header(&block_header, &qc, &Ed25519Verifier).unwrap();

// Install an externally trusted checkpoint after an epoch transition.
lc.update_validator_set(new_validator_set, new_height, new_hash);
```

Decode application proof bytes with `MptProof::from_bytes` or
`SmtProof::from_bytes`, then verify them against the expected key and an
independently trusted application root. Both proof types also support serde.
Choose the proof type matching the application's trie; MPT and SMT encodings
and roots are distinct.

```rust
use hotmint_light::{LightClient, MptProof, SmtProof};

let mpt = MptProof::from_bytes(&mpt_bytes)?;
assert!(LightClient::verify_state_proof(&mpt_root, &expected_key, &mpt)?);

let smt = SmtProof::from_bytes(&smt_bytes)?;
assert!(LightClient::verify_smt_state_proof(&smt_root, &expected_key, &smt)?);
```

## License

GPL-3.0-only
