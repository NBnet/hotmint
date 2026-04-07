# Hotmint False Positive Guide

Before reporting any finding, check it against this guide.

---

## FP-1: Rust Ownership System Already Prevents It

**Pattern**: Reporting memory safety issues in safe Rust code.
**Rule**: Only report memory safety inside `unsafe` blocks (only 2 exist: `libc::kill` in mgmt). The consensus path has zero unsafe.

## FP-2: Lock Held Across Entire Operation

**Pattern**: Reporting a TOCTOU race when the lock covers the full check-then-act.
**Rule**: Trace the `MutexGuard`/`RwLockGuard` lifetime. If held for the entire operation, no race.
**Important**: `parking_lot` locks MUST NOT be held across `.await` points. If you see a parking_lot guard live across an await, that IS a real bug (not a false positive).

## FP-3: Bounded Channel Backpressure is By Design

**Pattern**: Reporting that `mpsc::channel(64)` might drop messages when full.
**Rule**: Bounded channels are intentional backpressure. The sender handles `Full` (either by dropping the message or by blocking). Only report if:
1. The sender calls `.send().await` (blocking) while holding a lock (deadlock risk)
2. Critical consensus messages are silently dropped without retry

## FP-4: Unwrap/Expect on Known-Valid State

**Pattern**: Reporting `unwrap()` as potential panic.
**Rule**: Before reporting, verify whether the unwrap is:
1. On a value guaranteed by prior logic
2. In test code (acceptable)
3. On a channel `.recv()` where the sender is known to be alive
**When to report**: Only with a concrete scenario where the unwrap fails in production.

## FP-5: Clippy Would Catch It

**Pattern**: Reporting lints enforced by `cargo clippy -D warnings`.
**Rule**: CI enforces clippy. Focus on semantic correctness, not lint-level findings.

## FP-6: "Consider" Without Concrete Downside

**Pattern**: Advisory suggestions without a specific failure scenario.
**Rule**: Every finding must describe a concrete trigger and impact.

## FP-7: Test-Only Code Held to Production Standards

**Pattern**: Reporting issues in test/bench/example code.
**Rule**: Test code may use unwrap, hardcoded values, and simplified error handling. Only report if the test is incorrect.

## FP-8: Byzantine Behavior Handled by Protocol Design

**Pattern**: Reporting that a Byzantine node COULD send invalid messages.
**Rule**: BFT consensus assumes up to f Byzantine nodes. The protocol is DESIGNED to tolerate them. Only report Byzantine scenarios if:
1. The handling code has a bug that lets the attack succeed (e.g., missing signature check)
2. The attack can cause safety or liveness violations beyond the f-tolerance bound
**Do NOT report**: "A Byzantine node could send a bad message" — that's the whole point of BFT.

## FP-9: Performance Issue on Non-Consensus Path

**Pattern**: Reporting performance issues in startup, config parsing, or RPC handlers.
**Rule**: Hot paths are: vote processing, QC formation, block proposal, message dispatch. Cold paths include: startup, CLI, config, cluster management.

## FP-10: Epoch Transition Delay is By Design

**Pattern**: Reporting that validator set changes don't take effect immediately.
**Rule**: The +2 view delay is intentional — it ensures all honest nodes agree on the transition point. Only report if the delay is NOT +2, or if the delay logic is inconsistent between nodes.

## FP-11: Old Epoch Messages Accepted During Transition

**Pattern**: Reporting that messages from the previous epoch are still accepted.
**Rule**: During epoch transition, the previous epoch's validator set is intentionally retained for verification. Only report if messages from epoch E-2 (two epochs ago) are still accepted.

## FP-12: Stale QC/TC is Harmless

**Pattern**: Reporting that a QC from a past view is stored or forwarded.
**Rule**: Stale QCs are harmless — they cannot override a higher-view lock. They may be useful for catching up lagging nodes. Only report if a stale QC is used to ADVANCE state (e.g., commit a block).
