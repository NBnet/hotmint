# Network Subsystem Review Patterns

## Files
- `crates/hotmint-network/src/service.rs` — litep2p event loop, multi-protocol handling
- `crates/hotmint-network/src/` — peer management, protocol configuration

## Architecture
- litep2p P2P networking with 5 sub-protocols:
  1. Consensus notification (vote/block gossip)
  2. Mempool notification (tx gossip, max 512KB)
  3. Request-response (status, blocks, sync)
  4. Peer Exchange (PEX) for dynamic discovery
  5. Protocol isolation via `/hotmint/*` prefixes
- PeerMap: bidirectional ValidatorId ↔ PeerId mapping
- PeerBook: connection state tracking
- Per-sender message rate limiting (B-3 protection)

## Critical Invariants

### INV-N1: Message Authentication
Every inbound consensus message must have its signature verified against the current (or previous epoch's) validator set BEFORE any state mutation.
**Check**: Verify signature verification happens before message dispatch to consensus engine.

### INV-N2: PeerMap Bidirectional Consistency
If PeerMap says ValidatorId A maps to PeerId P, then the reverse map must say P maps to A.
**Check**: Verify all insert/remove operations update both maps atomically.

### INV-N3: Rate Limiting Bounds
Per-sender inbound message rate must be bounded to prevent Byzantine flood.
**Check**: Verify rate limiter is applied before message processing, not after.

### INV-N4: No Unbounded Buffers
All channels and queues must have bounded capacity. Backpressure or message drop on full.
**Check**: Grep for `mpsc::unbounded_channel` — should not exist on the consensus path.

## Common Bug Patterns

### Message Replay (technical-patterns.md 5.1)
Old messages re-injected into consensus engine.
**Check**: Verify message epoch/view filter at network layer or early in engine dispatch.

### PeerMap Stale After Reconnect
Validator reconnects with new PeerId but old mapping not cleaned up.
**Check**: Verify reconnection path removes old PeerId before inserting new one.

## Review Checklist
- [ ] Signature verified before consensus message dispatch
- [ ] PeerMap insert/remove is bidirectionally atomic
- [ ] Rate limiter applied on inbound path
- [ ] All channels bounded (no unbounded on consensus path)
- [ ] Stale peer entries cleaned on reconnect
- [ ] Protocol message size limits enforced (512KB mempool, etc.)
