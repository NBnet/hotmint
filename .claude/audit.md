# Audit Findings

> Auto-managed by /x-review and /x-fix.

## Open

*(No open findings)*

---

## Won't Fix

### [HIGH] light: BlockHeader fields not cryptographically bound to QC-verified hash
- **Where**: crates/hotmint-light/src/lib.rs:87-93
- **What**: verify_header checks qc.block_hash == header.hash but never verifies header fields produce that hash. Block::compute_hash() mixes in payload and evidence which BlockHeader omits. An attacker with a valid QC can craft a BlockHeader with arbitrary field values (e.g. forged app_hash) and set header.hash to the real hash.
- **Reason**: Requires architectural redesign — either adding a fields_hash to Block/BlockHeader and restructuring compute_hash() as hash(header_fields_hash || payload_hash || evidence_hash), or carrying the full payload/evidence hashes in BlockHeader. Both approaches change the block serialization format and affect many subsystems. Tracked for a future protocol version.

### [LOW] crypto: verify_batch() does not perform small-order public key rejection unlike verify_strict()
- **Where**: crates/hotmint-crypto/src/signer.rs:64 vs :109
- **What**: Individual verification uses verify_strict() which rejects small-order public keys. Batch verification uses ed25519_dalek::verify_batch() which does not. A validator with one of the 8 small-order Curve25519 points could produce signatures passing batch but failing strict verification.
- **Reason**: ed25519_dalek's verify_batch API does not expose a strict mode. A workaround (loop of verify_strict) trades ~2× batch throughput. Practical risk is negligible: only 8 small-order points exist, and validator registration controls key acceptance. Tracked for upstream ed25519-dalek enhancement.
