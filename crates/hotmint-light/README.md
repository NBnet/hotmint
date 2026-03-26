# hotmint-light

[![crates.io](https://img.shields.io/crates/v/hotmint-light.svg)](https://crates.io/crates/hotmint-light)
[![docs.rs](https://docs.rs/hotmint-light/badge.svg)](https://docs.rs/hotmint-light)

Light client verification library for the [Hotmint](https://github.com/rust-util-collections/hotmint) BFT consensus framework.

Verifies block headers and validator set transitions without replaying every block. A light client tracks the current validator set and verifies each new block's quorum certificate (QC) against it.

## Features

- **Header verification** — verify that a QC was signed by 2f+1 of the known validator set
- **Validator set tracking** — apply validator updates from committed blocks
- **Minimal trust** — only needs an initial trusted validator set, then follows the chain

## Usage

```rust
use hotmint_light::LightClient;

let mut lc = LightClient::new(trusted_validator_set);

// Verify a block header + QC
assert!(lc.verify_header(&block_hash, &commit_qc));

// Update validator set after epoch transition
lc.update_validator_set(new_validator_set);
```

## License

GPL-3.0-only
