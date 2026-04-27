use std::collections::HashMap;

use hotmint_types::Height;
use hotmint_types::validator::{ValidatorId, ValidatorSet};

/// Tracks validator liveness within an epoch by counting missed commit-QC
/// signatures.
///
/// **Deterministic**: All nodes derive liveness data from committed QC signer
/// bitfields stored on-chain, so the same set of offline validators is
/// reported deterministically at epoch boundaries.
pub struct LivenessTracker {
    /// Number of committed blocks each validator missed signing.
    missed: HashMap<ValidatorId, u64>,
    /// Total committed blocks tracked in this epoch.
    total_commits: u64,
    /// Threshold ratio (numerator / denominator). A validator is considered
    /// offline if `missed / total_commits > threshold_num / threshold_den`.
    threshold_num: u64,
    threshold_den: u64,
}

impl Default for LivenessTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl LivenessTracker {
    /// Create a new tracker. Default offline threshold: missed > 50% of commits.
    pub fn new() -> Self {
        Self {
            missed: HashMap::new(),
            total_commits: 0,
            threshold_num: 50,
            threshold_den: 100,
        }
    }

    /// Record a committed block's QC signer bitfield.
    ///
    /// `signers` is the commit-QC's signer bitfield (index-aligned with the
    /// validator set). Validators whose bit is `false` are counted as having
    /// missed this commit.
    pub fn record_commit(&mut self, validator_set: &ValidatorSet, signers: &[bool]) {
        self.total_commits += 1;
        for (idx, vi) in validator_set.validators().iter().enumerate() {
            let signed = signers.get(idx).copied().unwrap_or(false);
            if !signed {
                *self.missed.entry(vi.id).or_insert(0) += 1;
            }
        }
    }

    /// Return validators whose miss rate exceeds the offline threshold.
    ///
    /// Returns `(ValidatorId, missed_count, total_commits)` for each offline
    /// validator, suitable for the application to apply downtime slashing.
    pub fn offline_validators(&self) -> Vec<(ValidatorId, u64, u64)> {
        if self.total_commits == 0 {
            return vec![];
        }
        let mut result = Vec::new();
        for (&id, &missed) in &self.missed {
            // missed / total > threshold_num / threshold_den
            // ⟹ missed * threshold_den > threshold_num * total  (no floating point)
            if missed * self.threshold_den > self.threshold_num * self.total_commits {
                result.push((id, missed, self.total_commits));
            }
        }
        result.sort_by_key(|&(id, _, _)| id.0);
        result
    }

    /// Reset the tracker for a new epoch.
    pub fn reset(&mut self) {
        self.missed.clear();
        self.total_commits = 0;
    }

    /// Get liveness stats for a specific validator.
    pub fn stats(&self, id: ValidatorId) -> (u64, u64) {
        let missed = self.missed.get(&id).copied().unwrap_or(0);
        (missed, self.total_commits)
    }

    /// Current number of tracked commits.
    pub fn total_commits(&self) -> u64 {
        self.total_commits
    }
}

/// Offline evidence reported to the application at epoch boundaries.
#[derive(Debug, Clone)]
pub struct OfflineEvidence {
    pub validator: ValidatorId,
    pub missed_commits: u64,
    pub total_commits: u64,
    /// The height at which this evidence was produced (last committed block).
    pub evidence_height: Height,
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotmint_types::crypto::PublicKey;
    use hotmint_types::validator::{ValidatorInfo, ValidatorSet};

    fn make_vs(n: usize) -> ValidatorSet {
        let validators: Vec<ValidatorInfo> = (0..n)
            .map(|i| ValidatorInfo {
                id: ValidatorId(i as u64),
                public_key: PublicKey(vec![i as u8; 32]),
                power: 1,
            })
            .collect();
        ValidatorSet::new(validators)
    }

    #[test]
    fn test_all_sign() {
        let vs = make_vs(4);
        let mut tracker = LivenessTracker::new();
        for _ in 0..10 {
            tracker.record_commit(&vs, &[true, true, true, true]);
        }
        assert!(tracker.offline_validators().is_empty());
        assert_eq!(tracker.total_commits(), 10);
    }

    #[test]
    fn test_one_offline() {
        let vs = make_vs(4);
        let mut tracker = LivenessTracker::new();
        // V3 never signs
        for _ in 0..10 {
            tracker.record_commit(&vs, &[true, true, true, false]);
        }
        let offline = tracker.offline_validators();
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0].0, ValidatorId(3));
        assert_eq!(offline[0].1, 10); // missed all 10
    }

    #[test]
    fn test_threshold_boundary() {
        let vs = make_vs(4);
        let mut tracker = LivenessTracker::new();
        // V2 misses exactly 50% → NOT offline (threshold is >50%)
        for i in 0..10 {
            let v2_signs = i % 2 == 0;
            tracker.record_commit(&vs, &[true, true, v2_signs, true]);
        }
        assert!(tracker.offline_validators().is_empty());

        // V2 misses 6/10 = 60% → offline
        tracker.reset();
        for i in 0..10 {
            let v2_signs = i < 4; // signs first 4, misses last 6
            tracker.record_commit(&vs, &[true, true, v2_signs, true]);
        }
        let offline = tracker.offline_validators();
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0].0, ValidatorId(2));
    }

    #[test]
    fn test_reset() {
        let vs = make_vs(4);
        let mut tracker = LivenessTracker::new();
        for _ in 0..5 {
            tracker.record_commit(&vs, &[true, true, true, false]);
        }
        assert_eq!(tracker.total_commits(), 5);
        tracker.reset();
        assert_eq!(tracker.total_commits(), 0);
        assert!(tracker.offline_validators().is_empty());
    }
}
