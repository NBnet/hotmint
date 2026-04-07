# API & ABCI Subsystem Review Patterns

## Files
- `crates/hotmint-api/src/rpc.rs` — JSON-RPC (TCP)
- `crates/hotmint-api/src/http_rpc.rs` — HTTP + WebSocket (axum)
- `crates/hotmint-abci/src/` — IPC proxy, length-prefixed framing
- `crates/hotmint-abci-proto/` — protobuf definitions

## Architecture
- JSON-RPC over TCP (raw socket) and HTTP (axum)
- WebSocket subscriptions for block/tx events
- ABCI IPC via Unix socket + protobuf (length-prefixed frames)
- Rate limiting per IP with exponential backoff
- SharedBlockStore: `Arc<parking_lot::RwLock<Box<dyn BlockStore>>>`

## Critical Invariants

### INV-API1: Read-Only Access
RPC/API handlers must never mutate consensus state. They only read from BlockStore and application state.
**Check**: Verify API handlers only acquire read locks (`RwLock::read()`), never write locks.

### INV-API2: Rate Limiting
HTTP endpoints must enforce per-IP rate limits to prevent DoS.
**Check**: Verify rate limiter is applied as middleware (axum layer), not per-handler.

### INV-API3: ABCI Frame Integrity
IPC frames must be length-prefixed and validated. An incorrect length field could cause deserialization of garbage.
**Check**: Verify frame reader validates length before allocating buffer. Bound maximum frame size.

### INV-API4: Subscription Cleanup
WebSocket subscriptions must be cleaned up when the client disconnects.
**Check**: Verify `on_disconnect` or equivalent removes the subscription. No unbounded accumulation.

## Common Bug Patterns

### RwLock Write in API Handler
An API handler accidentally takes a write lock on SharedBlockStore, blocking consensus.
**Check**: Grep for `.write()` in API handler code — should only see `.read()`.

### Unbounded Frame Size
ABCI IPC reads a length field from the socket and allocates that many bytes. A malicious client sends length = u64::MAX.
**Check**: Verify maximum frame size is bounded (e.g., 16MB).

### Rate Limiter Bypass via WebSocket
HTTP rate limiting applied but WebSocket connections bypass it.
**Check**: Verify WebSocket upgrade also goes through rate limiting.

## Review Checklist
- [ ] API handlers only acquire read locks on BlockStore
- [ ] HTTP rate limiting applied as middleware
- [ ] ABCI IPC frame size bounded
- [ ] WebSocket subscriptions cleaned up on disconnect
- [ ] Rate limiting covers WebSocket, not just HTTP
- [ ] Protobuf deserialization validates field sizes
- [ ] No sensitive data exposed in error messages
