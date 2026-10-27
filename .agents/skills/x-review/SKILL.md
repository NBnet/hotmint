---
name: x-review
description: Review a hotmint diff, commit range, or the full codebase for HotStuff-2 safety, liveness, persistence, and concurrency defects, updating docs/audit.md only. Use only when the user explicitly invokes $x-review, or as the review phase of $x-overhaul.
---

# Hotmint Deep Regression Review

Resolve repository paths from the Git root; Markdown links are relative to
this file. Read invocation arguments from the user prompt.

High-signal review. Code is read-only; `docs/audit.md` is the only write
allowed. A standalone review ends with findings and registry changes; it
does not commit, push, or deploy, and does not authorize code repairs.
Use [x-fix](../x-fix/SKILL.md) to resolve recorded findings,
[x-commit](../x-commit/SKILL.md) for self-reviewing commits, and
[x-overhaul](../x-overhaul/SKILL.md) for the full review-fix-commit pipeline.

## Setup

Read first:
[technical-patterns.md](../../../docs/ai/technical-patterns.md) (bug pattern
reference),
[review-core.md](../../../docs/ai/review-core.md) (review methodology,
classification, invariants), and
[false-positive-guide.md](../../../docs/ai/false-positive-guide.md)
(candidate refutation).
For each affected subsystem, map changed files to the subsystem table below
and read the matching guide under
[patterns/](../../../docs/ai/patterns/).

| Crate path | Subsystem |
|------------|-----------|
| `crates/hotmint-consensus/src/` | consensus |
| `crates/hotmint-types/src/` | core types |
| `crates/hotmint-crypto/src/` | crypto |
| `crates/hotmint-storage/src/` | storage |
| `crates/hotmint-network/src/` | network |
| `crates/hotmint-mempool/src/` | mempool |
| `crates/hotmint-api/src/` | API |
| `crates/hotmint-abci/src/` | ABCI |
| `crates/hotmint-light/src/` | light client |
| `crates/hotmint-staking/src/` | staking |
| `crates/hotmint-mgmt/src/` | cluster mgmt |

## Input

Accept at most one review scope supplied in the user's request:

| Input | Scope | How |
|-------|-------|-----|
| *(empty)* | Latest commit | `git diff HEAD~1`, `git log -1` |
| `N` (integer) | Last N commits | `git diff HEAD~N`, `git log -N --oneline` |
| `all` | Full codebase audit | Full Audit Protocol below |
| `<commit hash>` | Specific commit | `git diff <hash>~1 <hash>` |
| `<hash1>..<hash2>` | Commit range | `git diff <hash1> <hash2>` |

For diff-based scopes, proceed to the Execution Protocol. For `all`, use the
Full Audit Protocol. Reject anything else; never guess a scope.

## Execution Protocol

### 1. Context & Classification

1. Record a worktree baseline (`git status --short`, `git rev-parse HEAD`).
2. Read the full diff; identify ALL affected subsystems via the table above.
3. Read the corresponding pattern guide for each affected subsystem.
4. Classify each change per the review-core methodology (safety, liveness,
   persistence, control flow, etc.).

### 2. Deep Regression Analysis

For each HIGH or CRITICAL classified change:

- Read at least 50 lines of surrounding context.
- Trace all call sites of changed functions.
- Check safety/liveness/persistence invariants from review-core.md.
- Boundary conditions: genesis, epoch boundary, n=4, empty block.
- Failure paths: does the pacemaker still fire? Does consensus state stay
  consistent?
- Concurrency: is a parking_lot guard held across `.await`? Channel
  capacity? Lock ordering?

Cross-reference every candidate with `technical-patterns.md` and try to
refute it with `false-positive-guide.md`; keep only concrete evidence.

### 3. Cross-Cutting Analysis

- Crash safety: kill -9 here — is state fsynced? Recovery correct?
- Performance: latency added to vote processing or QC formation?
- API compatibility: does this break existing Application trait
  implementations?

### 4. Code Style Enforcement

- No lint suppression (`#[allow(...)]` is forbidden).
- Prefer imports over inline paths (3+ uses → top-level `use`;
  function-body `use` is fine; 1–2 inline uses of common `std::` items are
  acceptable).
- Merge imports with common prefixes.
- Doc-code alignment: if public signatures, struct fields, module
  structure, or public types change, verify docs still match — `AGENTS.md`
  architecture table, `docs/ai/review-core.md` subsystem mappings, this
  skill's full-audit partitioning table, `docs/ai/patterns/` guides, doc
  comments, and README.

### 5. Unsafe Code Audit

The codebase has only three unsafe constructs, all in
`crates/hotmint-mgmt/src/local.rs` (two `libc::kill` call sites plus the
`unsafe extern "C"` declaration), none on the consensus path. For any added
`unsafe` block: require a SAFETY comment and check for UB.

### 6. Audit Registry (docs/audit.md)

1. Read `docs/audit.md` (create if absent).
2. **Prune**: verify each `## Open` entry against the current codebase;
   remove entries that are 100% fixed.
3. **Merge**: add new findings under `## Open`, deduplicating against
   existing entries, sorted CRITICAL → HIGH → MEDIUM → LOW.
4. **Preserve**: leave all `## Won't Fix` entries untouched.

File format:

```markdown
# Audit Findings

> Auto-managed by $x-review and $x-fix.

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

## Full Audit Protocol (`all` scope)

Audit all source files, not just uncommitted changes (~19K LOC excluding
tests, ~23K with tests). Partition the repository by subsystem; if
delegation is available and authorized, launch one read-only reviewer per
subsystem group and keep ownership disjoint.

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

Each reviewer reads all source files in its group, loads the pattern guide,
and performs deep analysis (invariants, boundary conditions, failure paths,
concurrency, unsafe audit). Aggregate and deduplicate findings, then update
`docs/audit.md` per the Audit Registry step.

Report:

```
## Full Audit Report

**Scope**: All crates (~19K LOC excluding tests, ~23K with tests)
**Subsystems Audited**: <list>
**Total Findings**: N (X critical, Y high, Z medium, W low)

## Findings
(sorted by severity, grouped by subsystem)

## Clean Areas
(subsystems with no findings — list what was checked)
```
