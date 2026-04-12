# Hotmint Review Core Methodology

This document defines the systematic review protocol for Hotmint code changes.

---

## Phase 1: Context Gathering

Before analyzing any change, gather context:

1. **Read the diff** — understand every changed line
2. **Identify affected subsystems** — map changes to:
   - `crates/hotmint-consensus/src/` → consensus (view protocol, state, vote, commit, pacemaker, sync)
   - `crates/hotmint-types/src/` → core types
   - `crates/hotmint-crypto/src/` → cryptography
   - `crates/hotmint-storage/src/` → storage, persistence
   - `crates/hotmint-network/src/` → networking
   - `crates/hotmint-mempool/src/` → mempool
   - `crates/hotmint-api/src/` → API
   - `crates/hotmint-abci/src/` → ABCI IPC
   - `crates/hotmint-light/src/` → light client
   - `crates/hotmint-staking/src/` → staking
   - `crates/hotmint-mgmt/src/` → cluster management
3. **Load subsystem patterns** — read the relevant `.claude/docs/patterns/<subsystem>.md`
4. **Check call sites** — use grep/LSP to find all callers of changed functions
5. **Check related tests** — identify which test files cover the changed code

## Phase 2: Change Classification

Classify each change into one or more categories:

| Category | Description | Risk Level |
|----------|-------------|------------|
| Safety property | Voting rules, QC validation, commit logic | CRITICAL |
| Liveness property | Pacemaker, view advancement, timeout | CRITICAL |
| Vote aggregation | Threshold, dedup, epoch/view check | CRITICAL |
| Epoch transition | Validator set changes, transition window | CRITICAL |
| Signature/crypto | Signing, verification, domain separation | CRITICAL |
| State persistence | WAL, crash recovery, consensus state save | HIGH |
| Network protocol | Message handling, peer management, rate limiting | HIGH |
| Async concurrency | tokio tasks, channels, locks, RwLock, select! | HIGH |
| Error handling | Result, Option, unwrap, expect, ? operator | MEDIUM |
| Mempool logic | Priority, RBF, eviction, ordering | MEDIUM |
| API/RPC | HTTP handlers, WebSocket, serialization | LOW |
| Configuration | Options, defaults, thresholds | LOW |
| Logging/metrics | tracing calls, stats updates | LOW |
| Test changes | New or modified test cases | LOW |

## Phase 3: Regression Analysis

For each HIGH or CRITICAL change, perform deep analysis:

### 3.1 Invariant Check

**Safety invariants (MUST hold — violation = consensus failure):**
- **Agreement**: No two honest nodes commit different blocks at the same height
- **Validity**: Every committed block was proposed by a valid leader
- **Locked QC rule**: Vote only for blocks extending locked block, or with QC.view > locked_view
- **2f+1 threshold**: QCs and TCs require exactly 2f+1 distinct valid signatures
- **Domain separation**: Every signed message includes chain_id + epoch + view
- **Commit finality**: Once a DC is formed, the block and all ancestors are final

**Liveness invariants (SHOULD hold — violation = cluster stalls):**
- **View progress**: Every honest node eventually advances to a new view
- **Timeout escalation**: Pacemaker timeout increases monotonically until cap
- **Sync convergence**: A lagging node eventually catches up
- **TC advancement**: 2f+1 timeouts trigger view advance without leader proposal

**Persistence invariants:**
- **Crash atomicity**: Consensus state is persisted before commit returns
- **WAL durability**: Commit intent is fsynced before application commit
- **Recovery correctness**: After restart, node resumes from last persisted state without re-voting

### 3.2 Boundary Condition Analysis
- Single validator (n=1, f=0)
- Minimum BFT cluster (n=4, f=1)
- Large validator set (n=100, f=33)
- First view of first epoch (genesis)
- Last view before epoch transition
- Empty block (no transactions)
- Maximum block size
- Concurrent view changes (multiple TC at same time)
- Node restart during active view

### 3.3 Failure Path Analysis
For every new error path introduced:
- Does the error path clean up all acquired resources?
- Does partial failure leave the consensus state consistent?
- Does the pacemaker still fire after an error?
- Is the error propagated without losing the current view?

### 3.4 Concurrency Analysis
For changes touching shared state:
- What lock is held? (parking_lot::RwLock, parking_lot::Mutex, tokio::sync::Mutex)
- Is the lock held across an `.await` point? (parking_lot MUST NOT be held across await)
- Channel capacity — bounded? What happens when full?
- Is the lock ordering consistent? (e.g., mempool: entries → seen)
- For `select!` branches: are all branches cancel-safe?

## Phase 4: Cross-Cutting Concerns

### 4.1 Crash Safety
If the change touches commit, state persistence, or WAL:
- What happens if `kill -9` hits at this exact line?
- Is consensus state fsynced before commit response?
- Can the node recover and resume without re-voting?

### 4.2 Performance Regression
- Does this add a lock to the consensus hot loop?
- Does this add serialization/deserialization overhead?
- Does this increase message size on the network?
- Does this affect vote aggregation latency?

### 4.3 API Contract
- Does the change alter observable behavior?
- Are new Application trait methods backward-compatible (have defaults)?
- Do new config options have sensible defaults?

### 4.4 Code Style Rules
Enforced project conventions — violations are findings (severity LOW):
- **No lint suppression**: `#[allow(...)]` is forbidden. Fix warnings at source.
- **Prefer imports over inline paths**: Avoid `std::foo::Bar::new()` inline in function bodies when the same path appears 3+ times in a file; add a `use` import at file top instead. Function-body `use` statements (scoped imports) are fine. 1-2 inline uses of common `std::` items are acceptable.
- **Grouped imports**: Merge common prefixes.
- **Doc-code alignment**: Public API changes must update corresponding docs. When a change adds, removes, or renames a public type, module, or subsystem path, also verify:
  - `CLAUDE.md` architecture table (paths, type names, dependency info)
  - `.claude/docs/review-core.md` subsystem path mappings
  - `.claude/commands/hm-review.md` full-audit subsystem partitioning table
  - `.claude/docs/patterns/` guides — referenced file lists and invariants

## Phase 5: Reporting

### Finding Format
```
[SEVERITY] subsystem: one-line summary

WHERE: file:line_range
WHAT: Description of the issue
WHY: Why this is a problem (reference invariant or pattern from technical-patterns.md)
FIX: Suggested fix (if clear) or questions to resolve
```

### Severity Levels
- **CRITICAL**: Safety violation, liveness violation, or cryptographic flaw
- **HIGH**: Incorrect state persistence, message handling bug, or concurrency issue
- **MEDIUM**: Edge case bug, error handling gap, or minor performance issue
- **LOW**: Style, clarity, or non-functional improvement
- **INFO**: Observation or question, not necessarily a bug

### Quality Gate
Only report findings where you have **concrete evidence** from the code. Never report:
- Hypothetical Byzantine attacks without a specific triggering sequence
- Style preferences not related to correctness
- "Consider" suggestions without a clear downside to the current code

Consult `.claude/docs/false-positive-guide.md` before finalizing any finding.
