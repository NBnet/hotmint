---
name: x-commit
description: Self-review all uncommitted hotmint changes, fix every finding, format, bump the patch version, and create a local commit. Use only when the user explicitly invokes $x-commit.
---

# Hotmint Self-Reviewing Commit

Resolve repository paths from the Git root; Markdown links are relative to
this file. Read invocation arguments from the user prompt.

This explicit invocation authorizes reviewing the uncommitted diff, fixing
findings, and creating one local commit. It does not authorize pushing,
tagging, or modifying other repositories.

## Setup

Read first:
[technical-patterns.md](../../../docs/ai/technical-patterns.md) (bug pattern
reference),
[review-core.md](../../../docs/ai/review-core.md) (review methodology), and
[false-positive-guide.md](../../../docs/ai/false-positive-guide.md)
(candidate refutation).

## Execution Protocol

### 1. Deep Self-Review

1. Record a baseline (`git status --short`, `git rev-parse HEAD`), then run
   `git diff HEAD` to collect all uncommitted changes.
2. If the diff is empty, report "nothing to commit" and stop.
3. Identify ALL affected subsystems via the crate-path table in
   [x-review](../x-review/SKILL.md) and read the matching guide under
   `docs/ai/patterns/` for each.
4. Perform the full regression analysis from `review-core.md`:
   - **Classify** each change (safety, liveness, persistence, control flow, etc.)
   - **Invariant check** — safety/liveness/persistence invariants
   - **Boundary conditions** — genesis, epoch boundary, n=4, empty block
   - **Failure paths** — pacemaker still fires? Consensus state consistent?
   - **Concurrency** — parking_lot guard across `.await`? Channel capacity?
     Lock ordering?
5. Check cross-cutting concerns: crash safety (fsync? recovery correct?),
   performance (vote processing / QC formation latency), API compatibility
   (Application trait changes).
6. Enforce style rules: no `#[allow(...)]`, imports over inline paths
   (3+ uses), grouped imports, doc-code alignment for public API changes.
7. Audit any added/modified `unsafe` blocks.
8. Refute every finding against `false-positive-guide.md` — retain only
   findings with concrete evidence.

### 2. Fix All Findings

For EVERY confirmed finding (CRITICAL, HIGH, MEDIUM, or LOW):

1. Fix completely — no TODOs, no "fix later", no partial fixes.
2. Re-run `git diff HEAD` and repeat the review on the new diff.
3. Fix any new findings introduced by the fixes. Iterate until clean.
4. Report the final list of fixes applied.

### 3. Format

Run `make fmt`.

### 4. Bump Patch Version — MANDATORY

1. Run `git diff HEAD --name-only` — if it lists any `.rs` file, a version
   bump is required. Skip only if every changed file is non-code (`.md`,
   version-only `.toml`, etc.).
2. Read `version = "X.Y.Z"` under `[workspace.package]` in `Cargo.toml`.
3. Compute `NEW = X.Y.(Z+1)` (e.g., `0.8.7` → `0.8.8`).
4. Set `version = "NEW"` under `[workspace.package]`.
5. Verify by grepping the file for the NEW version string; fix any mismatch
   before continuing.

### 5. Commit

1. Run `git diff HEAD --stat` and `git log -5 --oneline` for scope and
   commit style.
2. Draft a message following the repo's existing style (type prefix:
   `fix:`, `feat:`, `style:`, `refactor:`, etc.); summarize the "why",
   keep the subject to 1–2 sentences, add a body for multi-subsystem work.
3. Stage specific files with `git add` (not `-A`).
4. Commit with a HEREDOC — **no co-author line**:

```
git commit -m "$(cat <<'EOF'
<commit message here>
EOF
)"
```

5. Run `git status` to verify the commit succeeded.

Do not push, tag, amend, or rewrite history.

## Output Format

```
## Self-Review Commit Summary

**Reviewed**: <number of files changed>
**Subsystems**: <list>
**Findings**: <N found, N fixed> (or "0 — clean")
**Commit**: <short hash> <subject line>
```
