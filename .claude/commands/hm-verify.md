# Hotmint Finding Verification

You are verifying whether a reported code issue is a true bug or a false positive.

## Setup

1. Read `.claude/docs/false-positive-guide.md` — your primary reference.
2. Read `.claude/docs/technical-patterns.md` — for pattern matching.
3. Load relevant subsystem patterns from `.claude/docs/patterns/`.

## Input

A finding to verify — from `/hm-review`, code review, static analysis, or a hypothesis.

## Execution Protocol

### Step 1: Understand the Finding

1. Parse: what exactly is claimed to be wrong?
2. Identify subsystem and code location
3. Read actual code with full context (100+ lines)

### Step 2: False Positive Check

- [ ] **FP-1**: Safe Rust — memory safety prevented by compiler?
- [ ] **FP-2**: Lock held for entire operation — no TOCTOU?
- [ ] **FP-3**: Bounded channel backpressure is by design?
- [ ] **FP-4**: Unwrap on known-valid state?
- [ ] **FP-5**: Clippy would catch it?
- [ ] **FP-6**: "Consider" without concrete failure scenario?
- [ ] **FP-7**: Test-only code held to production standards?
- [ ] **FP-8**: Byzantine behavior handled by protocol design?
- [ ] **FP-9**: Performance issue on non-consensus path?
- [ ] **FP-10**: Epoch transition delay is by design (+2 views)?
- [ ] **FP-11**: Old epoch messages accepted during transition?
- [ ] **FP-12**: Stale QC/TC is harmless?

### Step 3: Reproduction Attempt

If finding passes FP check:
1. Construct a trigger scenario (specific validator count, view sequence)
2. Trace the code path with the scenario
3. Check existing tests
4. Evaluate: realistic in production? Requires Byzantine behavior?

### Step 4: Verdict

| Verdict | Meaning |
|---------|---------|
| **CONFIRMED** | Real bug, concrete trigger, should fix |
| **LIKELY** | Appears real, trigger hard to construct |
| **UNCERTAIN** | Needs more investigation or a test |
| **FALSE POSITIVE** | Incorrect — cite FP rule |
| **WON'T FIX** | Real but negligible risk or worse tradeoffs |

## Output Format

```
## Verification: <one-line summary>

**Verdict**: CONFIRMED / LIKELY / UNCERTAIN / FALSE POSITIVE / WON'T FIX

### False Positive Checklist
- FP-N: [PASS/FAIL] <reason>

### Analysis
<reasoning>

### Trigger Scenario
<if CONFIRMED/LIKELY: exact steps>
<if FALSE POSITIVE: why no trigger>

### Recommendation
<what to do next>
```
