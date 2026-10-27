use hotmint_types::{
    BlockHash, DoubleCertificate, Epoch, Height, QuorumCertificate, ValidatorId, ValidatorSet,
    ViewNumber, vote::VoteType,
};

/// Role of the validator in the current view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewRole {
    Leader,
    Replica,
}

/// Step within a view (state machine progression)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewStep {
    Entered,
    WaitingForStatus,
    Proposed,
    WaitingForProposal,
    Voted,
    CollectingVotes,
    Prepared,
    SentVote2,
    Done,
}

/// Mutable consensus state for a single validator
pub struct ConsensusState {
    pub validator_id: ValidatorId,
    pub validator_set: ValidatorSet,
    /// Blake3 hash of the chain identifier — included in all signing bytes
    /// to prevent cross-chain signature replay.
    pub chain_id_hash: [u8; 32],
    pub current_view: ViewNumber,
    pub role: ViewRole,
    pub step: ViewStep,
    pub locked_qc: Option<QuorumCertificate>,
    pub highest_double_cert: Option<DoubleCertificate>,
    pub highest_qc: Option<QuorumCertificate>,
    pub last_committed_height: Height,
    /// Application state root from the most recently committed block's execution.
    /// Included in the next proposed block for cross-node state divergence detection.
    pub last_app_hash: BlockHash,
    pub current_epoch: Epoch,
    /// Vote extensions from the most recent DoubleCertificate's Vote2 round.
    /// Set when a DC triggers a view advance; consumed by the next `create_payload`.
    pub pending_vote_extensions: Vec<(ValidatorId, Vec<u8>)>,
    /// Last Vote1 (phase-1 vote) this node signed, as `(view, block_hash)`.
    /// Persisted across restarts (privval-style) so a crash-recovered node
    /// never signs a conflicting vote at a view it already voted in.
    pub last_vote1: Option<(ViewNumber, BlockHash)>,
    /// Last Vote2 (phase-2 vote) this node signed, as `(view, block_hash)`.
    pub last_vote2: Option<(ViewNumber, BlockHash)>,
}

impl ConsensusState {
    pub fn new(validator_id: ValidatorId, validator_set: ValidatorSet) -> Self {
        Self::with_chain_id(validator_id, validator_set, "")
    }

    pub fn with_chain_id(
        validator_id: ValidatorId,
        validator_set: ValidatorSet,
        chain_id: &str,
    ) -> Self {
        let current_epoch = Epoch::genesis(validator_set.clone());
        let chain_id_hash = *blake3::hash(chain_id.as_bytes()).as_bytes();
        Self {
            validator_id,
            validator_set,
            chain_id_hash,
            current_view: ViewNumber::GENESIS,
            role: ViewRole::Replica,
            step: ViewStep::Entered,
            locked_qc: None,
            highest_double_cert: None,
            highest_qc: None,
            last_committed_height: Height::GENESIS,
            last_app_hash: BlockHash::GENESIS,
            current_epoch,
            pending_vote_extensions: vec![],
            last_vote1: None,
            last_vote2: None,
        }
    }

    /// Double-sign guard for locally produced votes.
    ///
    /// Records `(view, block_hash)` for the given vote type and returns
    /// whether signing may proceed:
    /// - re-signing the *same* block at the same view is allowed (ed25519 is
    ///   deterministic, so the signature is identical);
    /// - signing a *different* block at a view already voted in is refused
    ///   (this is what protects a crash-recovered node from amnesia
    ///   double-signing);
    /// - a vote for a view lower than the recorded one is also refused.
    ///
    /// Callers MUST persist the recorded vote to durable state BEFORE
    /// broadcasting the signed vote.
    pub fn check_and_record_vote(
        &mut self,
        vote_type: VoteType,
        view: ViewNumber,
        block_hash: BlockHash,
    ) -> bool {
        let slot = match vote_type {
            VoteType::Vote => &mut self.last_vote1,
            VoteType::Vote2 => &mut self.last_vote2,
        };
        match slot {
            Some((v, h)) if *v > view => false,
            Some((v, h)) if *v == view => *h == block_hash,
            _ => {
                *slot = Some((view, block_hash));
                true
            }
        }
    }

    pub fn is_leader(&self) -> bool {
        self.role == ViewRole::Leader
    }

    pub fn update_highest_qc(&mut self, qc: &QuorumCertificate) {
        let dominated = self.highest_qc.as_ref().is_none_or(|h| qc.view > h.view);
        if dominated {
            self.highest_qc = Some(qc.clone());
        }
    }

    pub fn update_locked_qc(&mut self, qc: &QuorumCertificate) {
        let dominated = self.locked_qc.as_ref().is_none_or(|h| qc.view > h.view);
        if dominated {
            self.locked_qc = Some(qc.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotmint_types::crypto::PublicKey;
    use hotmint_types::validator::{ValidatorInfo, ValidatorSet};

    fn test_state() -> ConsensusState {
        let vs = ValidatorSet::new(vec![ValidatorInfo {
            id: ValidatorId(0),
            public_key: PublicKey(vec![0; 32]),
            power: 1,
        }]);
        ConsensusState::new(ValidatorId(0), vs)
    }

    #[test]
    fn double_sign_guard_allows_first_and_identical_re_sign() {
        let mut state = test_state();
        let h1 = BlockHash([1; 32]);
        assert!(state.check_and_record_vote(VoteType::Vote, ViewNumber(1), h1));
        // Re-signing the same block at the same view is safe (deterministic).
        assert!(state.check_and_record_vote(VoteType::Vote, ViewNumber(1), h1));
    }

    #[test]
    fn double_sign_guard_refuses_conflicting_same_view_vote() {
        let mut state = test_state();
        let h1 = BlockHash([1; 32]);
        let h2 = BlockHash([2; 32]);
        assert!(state.check_and_record_vote(VoteType::Vote, ViewNumber(1), h1));
        // A different block at the same view is an amnesia double-sign.
        assert!(!state.check_and_record_vote(VoteType::Vote, ViewNumber(1), h2));
        // The recorded slot is unchanged after the refusal.
        assert_eq!(state.last_vote1, Some((ViewNumber(1), h1)));
    }

    #[test]
    fn double_sign_guard_refuses_lower_view_and_tracks_types_independently() {
        let mut state = test_state();
        let h1 = BlockHash([1; 32]);
        let h2 = BlockHash([2; 32]);
        assert!(state.check_and_record_vote(VoteType::Vote, ViewNumber(5), h1));
        assert!(!state.check_and_record_vote(VoteType::Vote, ViewNumber(4), h2));
        // Vote1 and Vote2 slots are tracked independently.
        assert!(state.check_and_record_vote(VoteType::Vote2, ViewNumber(5), h1));
        assert!(!state.check_and_record_vote(VoteType::Vote2, ViewNumber(5), h2));
        // A later view records normally.
        assert!(state.check_and_record_vote(VoteType::Vote, ViewNumber(6), h2));
    }
}
