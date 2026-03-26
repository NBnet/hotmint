use std::collections::HashSet;
use std::path::Path;

use hotmint_consensus::evidence_store::EvidenceStore;
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::validator::ValidatorId;
use hotmint_types::view::ViewNumber;
use ruc::*;
use vsdb::MapxOrd;

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
        // C-5: Prune committed proofs to avoid unbounded growth.
        self.proofs
            .retain(|p| !self.committed.contains(&(p.view, p.validator)));
    }

    fn all(&self) -> Vec<EquivocationProof> {
        self.proofs.clone()
    }
}

// ---- Persistent vsdb-backed evidence store (C-3) ----

const META_FILE: &str = "evidence_store.meta";

/// Persistent evidence store backed by vsdb.
///
/// Proofs survive node restarts. Uses a `MapxOrd<u64, EquivocationProof>` for
/// the proof list (keyed by auto-increment ID) and a `MapxOrd<u64, u8>` as a
/// committed-set (keyed by a hash of (view, validator)).
pub struct PersistentEvidenceStore {
    proofs: MapxOrd<u64, EquivocationProof>,
    committed: MapxOrd<u64, u8>,
    next_id: u64,
    meta_path: std::path::PathBuf,
}

impl PersistentEvidenceStore {
    /// Open an existing store or create a new one.
    /// Must be called after `vsdb::vsdb_set_base_dir`.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let meta_path = data_dir.join(META_FILE);
        if meta_path.exists() {
            let bytes = std::fs::read(&meta_path).c(d!("read evidence_store.meta"))?;
            if bytes.len() != 24 {
                return Err(eg!(
                    "corrupt evidence_store.meta: expected 24 bytes, got {}",
                    bytes.len()
                ));
            }
            let proofs_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
            let committed_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
            let next_id = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
            let proofs = MapxOrd::from_meta(proofs_id).c(d!("restore proofs"))?;
            let committed = MapxOrd::from_meta(committed_id).c(d!("restore committed"))?;
            Ok(Self {
                proofs,
                committed,
                next_id,
                meta_path: meta_path.clone(),
            })
        } else {
            let proofs: MapxOrd<u64, EquivocationProof> = MapxOrd::new();
            let committed: MapxOrd<u64, u8> = MapxOrd::new();
            let proofs_id = proofs.save_meta().c(d!())?;
            let committed_id = committed.save_meta().c(d!())?;
            let next_id = 0u64;
            let mut meta = Vec::with_capacity(24);
            meta.extend_from_slice(&proofs_id.to_le_bytes());
            meta.extend_from_slice(&committed_id.to_le_bytes());
            meta.extend_from_slice(&next_id.to_le_bytes());
            std::fs::write(&meta_path, &meta).c(d!("write evidence_store.meta"))?;
            Ok(Self {
                proofs,
                committed,
                next_id,
                meta_path,
            })
        }
    }

    fn committed_key(view: ViewNumber, validator: ValidatorId) -> u64 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&view.as_u64().to_le_bytes());
        hasher.update(&validator.0.to_le_bytes());
        let hash = hasher.finalize();
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
    }

    fn is_duplicate(&self, proof: &EquivocationProof) -> bool {
        self.proofs
            .iter()
            .any(|(_, p)| p.view == proof.view && p.validator == proof.validator)
    }

    /// A-4: Write the updated next_id back to the meta file so it survives restarts.
    fn persist_next_id(&self) {
        if let Ok(bytes) = std::fs::read(&self.meta_path)
            && bytes.len() == 24
        {
            let mut meta = bytes;
            meta[16..24].copy_from_slice(&self.next_id.to_le_bytes());
            let _ = std::fs::write(&self.meta_path, &meta);
        }
    }
}

impl EvidenceStore for PersistentEvidenceStore {
    fn put_evidence(&mut self, proof: EquivocationProof) {
        if self.is_duplicate(&proof) {
            return;
        }
        self.proofs.insert(&self.next_id, &proof);
        self.next_id += 1;
        // A-4: Persist next_id so it survives restarts.
        self.persist_next_id();
    }

    fn get_pending(&self) -> Vec<EquivocationProof> {
        self.proofs
            .iter()
            .filter_map(|(_, p)| {
                let key = Self::committed_key(p.view, p.validator);
                if self.committed.get(&key).is_some() {
                    None
                } else {
                    Some(p)
                }
            })
            .collect()
    }

    fn mark_committed(&mut self, view: ViewNumber, validator: ValidatorId) {
        let key = Self::committed_key(view, validator);
        self.committed.insert(&key, &1);
        // C-5: Prune the committed proof from the proofs map to bound growth.
        let to_remove: Vec<u64> = self
            .proofs
            .iter()
            .filter(|(_, p)| p.view == view && p.validator == validator)
            .map(|(id, _)| id)
            .collect();
        for id in to_remove {
            self.proofs.remove(&id);
        }
    }

    fn all(&self) -> Vec<EquivocationProof> {
        self.proofs.iter().map(|(_, p)| p).collect()
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
        // C-5: committed proofs are pruned, so all() returns only the remaining one
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn deduplication() {
        let mut store = MemoryEvidenceStore::new();
        store.put_evidence(dummy_proof(1, 0));
        store.put_evidence(dummy_proof(1, 0)); // duplicate
        assert_eq!(store.all().len(), 1);
    }
}
