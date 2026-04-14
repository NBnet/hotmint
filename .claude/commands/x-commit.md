# Self-Reviewing Commit for Hotmint

You are performing a self-reviewing commit: review all uncommitted changes, fix every issue found, format, and commit.

## Setup

1. **MANDATORY**: Read `.claude/docs/technical-patterns.md` — bug pattern reference.
2. Read `.claude/docs/review-core.md` — review methodology.
3. Read `.claude/docs/false-positive-guide.md` — consult before reporting any finding.

## Execution Protocol

### Task 1: Deep Self-Review

1. Run `git diff HEAD` to collect all uncommitted changes.
2. If the diff is empty, report "nothing to commit" and stop.
3. Identify ALL affected subsystems by mapping changed files:
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
4. For EACH affected subsystem, read the corresponding pattern file from `.claude/docs/patterns/`.
5. Perform the full regression analysis from review-core.md:
   - **Classify** each change (safety, liveness, persistence, control flow, etc.)
   - **Invariant check** — verify safety/liveness/persistence invariants
   - **Boundary conditions** — genesis, epoch boundary, n=4, empty block
   - **Failure paths** — pacemaker still fires? Consensus state consistent?
   - **Concurrency** — parking_lot guard across `.await`? Channel capacity? Lock ordering?
6. Check cross-cutting concerns:
   - **Crash safety** — is state fsynced? Recovery correct?
   - **Performance** — vote processing or QC formation latency
   - **API compatibility** — Application trait changes
7. Enforce code style rules:
   - No `#[allow(...)]` — fix warnings at the source
   - Prefer imports over inline paths (3+ uses)
   - Grouped imports with common prefixes
   - Doc-code alignment for public API changes
8. Audit any added/modified `unsafe` blocks.
9. Cross-reference every finding with `false-positive-guide.md` — only retain findings with **concrete evidence**.

### Task 2: Fix All Findings

For EVERY finding from Task 1 (CRITICAL, HIGH, MEDIUM, or LOW):

1. Fix the issue completely — no TODOs, no "fix later", no partial fixes.
2. After all fixes are applied, re-run `git diff HEAD` and repeat Task 1 analysis on the new diff.
3. If new findings emerge from the fixes, fix those too. Iterate until clean.
4. Report the final list of fixes applied.

### Task 3: Format

1. Run `make fmt` to apply code formatting.

### Task 4: Bump Patch Version

If `git diff HEAD` shows actual code changes (not just version bumps):

1. Read the current version from `Cargo.toml` (the workspace `version = "x.y.z"` line under `[workspace.package]`).
2. Increment `z` by 1 (e.g., `0.8.7` → `0.8.8`).
3. Update `Cargo.toml` — `version = "x.y.z+1"`.

### Task 5: Commit

1. Run `git diff HEAD --stat` and `git log -5 --oneline` to understand scope and commit style.
2. Draft a commit message:
   - Follow the repo's existing commit message style (type prefix: `fix:`, `feat:`, `style:`, `refactor:`, etc.)
   - Summarize the "why" not the "what" — keep it concise (1-2 sentences for the subject)
   - Add a body with key details if the change spans multiple subsystems
3. Stage the relevant files with `git add` (specific files, not `-A`).
4. Commit using a HEREDOC — **do NOT include any co-author line**:

```
git commit -m "$(cat <<'EOF'
<commit message here>
EOF
)"
```

5. Run `git status` to verify the commit succeeded.

## Output Format

```
## Self-Review Commit Summary

**Reviewed**: <number of files changed>
**Subsystems**: <list>
**Findings**: <N found, N fixed> (or "0 — clean")
**Commit**: <short hash> <subject line>
```
