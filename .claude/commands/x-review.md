# Deep Regression Analysis for Hotmint

You are performing a deep code review of changes to Hotmint, a HotStuff-2 BFT consensus engine.

## Setup

1. **MANDATORY**: Read `.claude/docs/technical-patterns.md` first — your bug pattern reference.
2. Read `.claude/docs/review-core.md` — your review methodology.
3. Read `.claude/docs/false-positive-guide.md` — consult before reporting any finding.

## Input

Arguments: `$ARGUMENTS`

| Input | Scope | How |
|-------|-------|-----|
| *(empty)* | Latest commit | `git diff HEAD~1`, `git log -1` |
| `N` (integer) | Last N commits | `git diff HEAD~N`, `git log -N --oneline` |
| `all` | Full codebase audit | Read all source by subsystem (see Full Audit Protocol) |
| `<commit hash>` | Specific commit | `git diff <hash>~1 <hash>` |
| `<hash1>..<hash2>` | Commit range | `git diff <hash1> <hash2>` |

For diff-based reviews, proceed to Execution Protocol. For `all`, skip to Full Audit Protocol.

## Execution Protocol

### Task 1: Context & Classification

1. Read the full diff carefully
2. Identify ALL affected subsystems by mapping changed files:
   - `crates/hotmint-consensus/src/` → consensus
   - `crates/hotmint-types/src/` → core types
   - `crates/hotmint-crypto/src/` → crypto
   - `crates/hotmint-storage/src/` → storage
   - `crates/hotmint-network/src/` → network
   - `crates/hotmint-mempool/src/` → mempool
   - `crates/hotmint-api/src/` → API
   - `crates/hotmint-abci/src/` → ABCI
   - `crates/hotmint-light/src/` → light client
   - `crates/hotmint-staking/src/` → staking
   - `crates/hotmint-mgmt/src/` → cluster mgmt
3. For EACH affected subsystem, read the corresponding pattern file from `.claude/docs/patterns/`
4. Classify each change per the review-core methodology

### Task 2: Deep Regression Analysis

For each HIGH or CRITICAL classified change:

1. **Read surrounding code** — at least 50 lines of context
2. **Trace call sites** — find all callers of changed functions
3. **Check invariants** — verify safety, liveness, persistence invariants from review-core.md
4. **Boundary conditions** — check edge cases (genesis, epoch boundary, n=4, empty block)
5. **Failure paths** — does the pacemaker still fire? Does consensus state remain consistent?
6. **Concurrency** — is a parking_lot guard held across `.await`? Channel capacity? Lock ordering?

For each finding: cross-reference with `technical-patterns.md` and `false-positive-guide.md`.

### Task 3: Cross-Cutting Analysis

1. **Crash safety** — what if kill -9 hits here? Is state fsynced?
2. **Performance** — does this add latency to vote processing or QC formation?
3. **API compatibility** — does this break existing Application trait implementations?

### Task 4: Code Style Enforcement

1. **No lint suppression** — `#[allow(...)]` is forbidden.
2. **Prefer imports over inline paths** — Avoid inline `std::foo::Bar::new()` when the same path appears 3+ times in a file. Function-body `use` (scoped imports) are fine. 1-2 inline uses are acceptable.
3. **Import grouping** — merge common prefixes.
4. **Doc-code alignment** — If the change modifies a public function signature, struct field, module structure, or adds/removes/renames a public type or module, verify docs still match. Specifically check:
   - `CLAUDE.md` architecture table (subsystem paths, type names, dependency info)
   - `.claude/docs/review-core.md` subsystem path mappings
   - `.claude/commands/x-review.md` full-audit subsystem partitioning table
   - `.claude/docs/patterns/` guides — referenced file lists and invariants
   - Doc comments and README

### Task 5: Unsafe Code Audit

If any `unsafe` block is added (currently only 2 in mgmt):
1. Verify SAFETY comment exists
2. Check for UB

### Task 6: Audit Registry (.claude/audit.md)

After completing the analysis:

1. Read `.claude/audit.md` from the project root (create if absent).
2. **Prune**: For each entry under `## Open`, verify against the current codebase. Remove entries that are 100% fixed.
3. **Merge**: Add new findings from this review under `## Open`, deduplicating against existing entries. Sort by severity (CRITICAL → HIGH → MEDIUM → LOW).
4. **Preserve**: Leave all `## Won't Fix` entries untouched.
5. Write the updated `.claude/audit.md`.

The file format:

```markdown
# Audit Findings

> Auto-managed by /x-review and /x-fix.

## Open

### [SEVERITY] subsystem: one-line summary
- **Where**: file:line_range
- **What**: description
- **Why**: invariant/pattern violated
- **Suggested fix**: how to fix

---

## Won't Fix

### [SEVERITY] subsystem: one-line summary
- **Where**: file:line_range
- **What**: description
- **Reason**: why this cannot or should not be fixed
```

## Output Format

```
## Review Summary

**Commit**: <hash> <subject>
**Subsystems**: <list>
**Risk Level**: CRITICAL / HIGH / MEDIUM / LOW

## Findings

### [SEVERITY] subsystem: one-line summary

**Where**: file:line_range
**What**: Description
**Why**: Invariant/pattern violated (cite technical-patterns.md)
**Fix**: Suggested fix

## No Issues Found

(list areas checked)
```

---

## Full Audit Protocol (for `all` mode)

### Strategy: Parallel Subsystem Audit

Launch one Agent per subsystem group in parallel.

### Subsystem Partitioning

| Subsystem | Crate(s) | Pattern Guide |
|-----------|----------|---------------|
| consensus core | `hotmint-consensus` (view_protocol, state, commit, vote_collector) | `consensus.md` |
| pacemaker & sync | `hotmint-consensus` (pacemaker, sync) | `consensus.md` |
| consensus engine | `hotmint-consensus` (engine.rs) | `consensus.md` |
| types & crypto | `hotmint-types`, `hotmint-crypto` | `crypto.md` |
| storage | `hotmint-storage` | `storage.md` |
| network | `hotmint-network` | `network.md` |
| mempool | `hotmint-mempool` | `mempool.md` |
| API & ABCI | `hotmint-api`, `hotmint-abci`, `hotmint-abci-proto` | `api.md` |
| light & staking | `hotmint-light`, `hotmint-staking` | `crypto.md` |
| facade & mgmt | `hotmint`, `hotmint-mgmt` | (cross-cutting) |

### Aggregation

```
## Full Audit Report

**Scope**: All crates (~16K LOC)
**Subsystems Audited**: <list>
**Total Findings**: N (X critical, Y high, Z medium, W low)

## Findings
(sorted by severity, grouped by subsystem)

## Clean Areas
(subsystems with no findings — list what was checked)
```
