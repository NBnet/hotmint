# Full Codebase Audit-Fix-Commit Pipeline

You are performing a full codebase audit: review ALL source files (not just uncommitted changes), fix every finding, and commit.

## Phase 1: Full Codebase Review

Execute `/x-review all` — the full audit protocol.

1. Read the Setup section of `.claude/commands/x-review.md` and load all required documentation.
2. Perform the **Full Audit Protocol** (the `all` mode section at the end of x-review.md):
   - Launch parallel agents per subsystem
   - Each agent reads all source files in its subsystem, loads the corresponding pattern guide
   - Perform deep analysis: invariants, boundary conditions, failure paths, concurrency, unsafe audit
3. Aggregate and deduplicate all findings.
4. Manage `.claude/audit.md` — prune fixed entries, merge new findings sorted by severity.

## Phase 2: Fix & Commit

Execute the full `/x-fix` protocol (including its Self-Review and Commit phases).

1. Read `.claude/audit.md`.
2. Execute every task in `.claude/commands/x-fix.md` (Phase 1, 2, and 3).
3. Iterate until `.claude/audit.md` has zero open entries (or only Won't Fix).

## Output Format

```
## Full Audit Pipeline Summary

### Review (full codebase)
**Subsystems audited**: <list>
**Total findings**: N (X critical, Y high, Z medium, W low)

### Fix & Commit
**Fixed**: X | **Won't Fix**: Y | **Remaining**: 0
**Commit**: <short hash> <subject line>
```
