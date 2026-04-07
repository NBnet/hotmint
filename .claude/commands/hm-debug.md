# Hotmint Crash & Liveness Debugger

You are debugging a crash, liveness failure, safety violation, or incorrect behavior in Hotmint.

## Setup

1. **MANDATORY**: Read `.claude/docs/technical-patterns.md` — your bug pattern reference.
2. Read `.claude/docs/review-core.md` — methodology for systematic analysis.
3. After initial analysis, load relevant subsystem patterns from `.claude/docs/patterns/`.

## Input

The user will provide one or more of:
- A panic/crash backtrace
- A failing test case or reproduction steps
- A description of incorrect behavior (e.g., "cluster stops making progress after epoch transition")
- Logs showing consensus messages
- A node that is stuck or producing conflicting blocks

## Execution Protocol

### Task 1: Symptom Classification

| Symptom | Likely Categories |
|---------|-------------------|
| Cluster stops committing blocks | 2.x Liveness (pacemaker, view deadlock, sync loop) |
| Two different blocks committed at same height | 1.x Safety (equivocation, locked QC bypass) |
| Node panics/crashes | 6.x Storage, 3.x Vote Collection, async runtime |
| Node falls behind and can't catch up | 2.3 Sync Loop, 6.x Storage |
| Wrong validator set after epoch change | 4.x Epoch Transition |
| Messages not delivered | 5.x Networking (peer map, authentication) |
| Light client rejects valid header | 7.x Cryptographic (batch verify, domain separation) |
| Slashing evidence lost | 6.3 Evidence Store |
| Tx not included in block | Mempool (priority, eviction, RBF) |

### Task 2: Root Cause Investigation

**For safety violations:**
1. Identify the two conflicting committed blocks
2. Trace both commit paths — which QCs/DCs were used?
3. Check voting rules — did any honest node vote for both forks?
4. Check locked QC — was the locking rule bypassed?
5. Check epoch — did both blocks use the same validator set?

**For liveness failures:**
1. Check pacemaker — is the timeout firing?
2. Check view numbers — are nodes in the same view?
3. Check TC — are enough timeout messages being exchanged?
4. Check network — are messages being delivered?
5. Check sync — is the lagging node making progress?

**For crashes:**
1. Parse the backtrace
2. Identify the immediate cause (unwrap, index, channel closed)
3. Trace backwards: what state led here?
4. Check if crash is during steady-state or during transition (epoch, sync, restart)

**For concurrency issues:**
1. Check for parking_lot guards held across .await
2. Check channel capacities and sender/receiver lifecycles
3. Check lock ordering (mempool: entries → seen)
4. Check tokio::select! cancel safety

### Task 3: Hypothesis Verification

For each hypothesis:
1. "If X is correct, then Y should be true"
2. Verify Y by reading code, running tests, or checking logs
3. If Y is false, discard and try next
4. If Y is true, look for additional confirming evidence

### Task 4: Fix Proposal

1. Propose a minimal fix with exact code changes
2. Explain which invariant it restores
3. Identify needed regression tests
4. Check if the same pattern exists elsewhere

## Output Format

```
## Debug Report

**Symptom**: <one-line description>
**Root Cause**: <one-line description>
**Category**: <reference to technical-patterns.md>
**Severity**: CRITICAL / HIGH / MEDIUM

### Investigation
<step-by-step>

### Root Cause Detail
**Where**: file:line_range
**What**: <explanation>
**Trigger**: <exact conditions>

### Proposed Fix
<code diff or description>

### Regression Test
<test case>

### Related Code
<other locations with same pattern>
```
