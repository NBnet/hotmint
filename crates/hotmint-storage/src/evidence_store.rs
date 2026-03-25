use std::collections::HashSet;

use hotmint_consensus::evidence_store::EvidenceStore;
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::validator::ValidatorId;
use hotmint_types::view::ViewNumber;

/// In-memory evidence store backed by a `Vec` and a committed-set.
pub struct MemoryEvidenceStore {
    proofs: Vec<EquivocationProof>,
    committed: HashSet<(ViewNumber, ValidatorId)>,
}

impl MemoryEvidenceStore {
    pub fn new() -> Self {
        Self {
            proofs: Vec::new(),
            committed: HashSet::new(),
        }
    }
}

impl Default for MemoryEvidenceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl EvidenceStore for MemoryEvidenceStore {
    fn put_evidence(&mut self, proof: EquivocationProof) {
        // Deduplicate: skip if we already have evidence for this (view, validator).
        let dominated = self
            .proofs
            .iter()
            .any(|p| p.view == proof.view && p.validator == proof.validator);
        if !dominated {
            self.proofs.push(proof);
        }
    }

    fn get_pending(&self) -> Vec<EquivocationProof> {
        self.proofs
            .iter()
            .filter(|p| !self.committed.contains(&(p.view, p.validator)))
            .cloned()
            .collect()
    }

    fn mark_committed(&mut self, view: ViewNumber, validator: ValidatorId) {
        self.committed.insert((view, validator));
    }

    fn all(&self) -> Vec<EquivocationProof> {
        self.proofs.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotmint_types::block::BlockHash;
    use hotmint_types::crypto::Signature;
    use hotmint_types::vote::VoteType;

    fn dummy_proof(view: u64, validator: u64) -> EquivocationProof {
        EquivocationProof {
            validator: ValidatorId(validator),
            view: ViewNumber(view),
            vote_type: VoteType::Vote,
            epoch: Default::default(),
            block_hash_a: BlockHash::GENESIS,
            signature_a: Signature(vec![1]),
            block_hash_b: BlockHash::GENESIS,
            signature_b: Signature(vec![2]),
        }
    }

    #[test]
    fn put_and_get_pending() {
        let mut store = MemoryEvidenceStore::new();
        store.put_evidence(dummy_proof(1, 0));
        store.put_evidence(dummy_proof(2, 1));

        assert_eq!(store.get_pending().len(), 2);
        assert_eq!(store.all().len(), 2);
    }

    #[test]
    fn mark_committed_filters_pending() {
        let mut store = MemoryEvidenceStore::new();
        store.put_evidence(dummy_proof(1, 0));
        store.put_evidence(dummy_proof(2, 1));
        store.mark_committed(ViewNumber(1), ValidatorId(0));

        let pending = store.get_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].view, ViewNumber(2));
        // all() still returns everything
        assert_eq!(store.all().len(), 2);
    }

    #[test]
    fn deduplication() {
        let mut store = MemoryEvidenceStore::new();
        store.put_evidence(dummy_proof(1, 0));
        store.put_evidence(dummy_proof(1, 0)); // duplicate
        assert_eq!(store.all().len(), 1);
    }
}
