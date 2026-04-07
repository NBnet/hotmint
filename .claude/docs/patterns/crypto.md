# Cryptographic Subsystem Review Patterns

## Files
- `crates/hotmint-crypto/src/lib.rs` — Ed25519 signing/verification, Blake3 hashing
- `crates/hotmint-types/src/` — signable types, domain separation
- `crates/hotmint-light/src/lib.rs` — batch signature verification

## Architecture
- Ed25519 (ed25519-dalek) for all signatures
- Blake3 for non-cryptographic hashing (block hash, tx hash)
- Domain-separated signing: payload includes chain_id_hash + epoch + view + message_type + block_hash
- Batch verification for light client header checks

## Critical Invariants

### INV-CR1: Domain Separation Completeness
Every signable message must include: `chain_id_hash || epoch || view || message_type || payload_hash`.
**Check**: Verify the "to_signable_bytes" method for each message type includes all domain fields. Missing any field enables cross-chain/cross-epoch/cross-type replay.

### INV-CR2: Signature Strictness
Ed25519 verification must reject non-canonical signatures (malleability protection).
**Check**: Verify ed25519-dalek is used without disabling strict mode. Default is strict.

### INV-CR3: Batch Verification Soundness
Batch verification must return true ONLY if ALL signatures in the batch are valid.
**Check**: Verify the batch verify API is not short-circuiting on first valid.

### INV-CR4: Hash Collision Resistance
Block hash and tx hash use Blake3 which provides collision resistance. If any code path uses a weaker hash or truncated hash as an identifier, collisions become practical.
**Check**: Verify no truncation of Blake3 output for identity purposes.

### INV-CR5: Key Derivation Determinism
ValidatorId must be deterministically derived from the public key. Non-determinism would cause different nodes to disagree on validator identity.
**Check**: Verify ValidatorId = hash(pubkey) or pubkey bytes directly.

## Common Bug Patterns

### Missing Chain ID in Signature (technical-patterns.md 7.1)
Vote signed without chain_id. Replayed on another chain with same validator set.
**Check**: Verify chain_id_hash is the FIRST field in the signed payload.

### Batch Verify Returns True on Partial Success (technical-patterns.md 7.2)
Light client accepts header if 1 of N signatures verifies.
**Check**: Verify batch API specification — ed25519-dalek's `verify_batch` is all-or-nothing.

### Signature Malleability (technical-patterns.md 7.3)
Two different byte sequences verify for the same (key, msg). Equivocation detection missed because signatures differ but votes are semantically identical.
**Check**: Verify dedup is by (validator_id, view, block_hash), NOT by signature bytes.

## Review Checklist
- [ ] Domain separation includes chain_id, epoch, view, message_type
- [ ] Ed25519 strict mode (reject non-canonical S values)
- [ ] Batch verification is all-or-nothing
- [ ] No Blake3 output truncation for identity
- [ ] ValidatorId derivation is deterministic
- [ ] Equivocation detection by semantic content, not signature bytes
