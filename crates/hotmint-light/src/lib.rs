//! Light client verification for Hotmint BFT consensus.
//!
//! Verifies block headers using QC signatures without downloading full blocks.
//! Also provides MPT state proof verification via [`LightClient::verify_state_proof`].

pub use vsdb::MptProof;

use ruc::*;
use std::sync::{Mutex, MutexGuard};

use hotmint_crypto::has_quorum;
use hotmint_types::block::{Block, BlockHash, Height};
use hotmint_types::certificate::QuorumCertificate;
use hotmint_types::crypto::Verifier;
use hotmint_types::validator::{ValidatorId, ValidatorSet};
use hotmint_types::view::ViewNumber;
use hotmint_types::vote::{Vote, VoteType};

/// Lightweight version of Block without the payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockHeader {
    pub height: Height,
    pub parent_hash: BlockHash,
    pub view: ViewNumber,
    pub proposer: ValidatorId,
    pub timestamp: u64,
    pub app_hash: BlockHash,
    pub hash: BlockHash,
}

impl From<&Block> for BlockHeader {
    fn from(block: &Block) -> Self {
        Self {
            height: block.height,
            parent_hash: block.parent_hash,
            view: block.view,
            proposer: block.proposer,
            timestamp: block.timestamp,
            app_hash: block.app_hash,
            hash: block.hash,
        }
    }
}

/// Light client that verifies block headers against a trusted validator set.
pub struct LightClient {
    trusted_validator_set: ValidatorSet,
    trusted_state: Mutex<TrustedState>,
    chain_id_hash: [u8; 32],
}

#[derive(Debug, Clone)]
struct TrustedState {
    height: Height,
    hash: Option<BlockHash>,
}

impl LightClient {
    /// Create a new light client with a trusted validator set and height.
    ///
    /// This legacy constructor has a trusted hash only at genesis. For any
    /// non-genesis checkpoint, use [`Self::new_with_trusted_hash`].
    pub fn new(
        trusted_validator_set: ValidatorSet,
        trusted_height: Height,
        chain_id_hash: [u8; 32],
    ) -> Self {
        let trusted_hash = if trusted_height == Height::GENESIS {
            Some(BlockHash::GENESIS)
        } else {
            None
        };
        Self {
            trusted_validator_set,
            trusted_state: Mutex::new(TrustedState {
                height: trusted_height,
                hash: trusted_hash,
            }),
            chain_id_hash,
        }
    }

    /// Create a light client from an explicit trusted block checkpoint.
    pub fn new_with_trusted_hash(
        trusted_validator_set: ValidatorSet,
        trusted_height: Height,
        trusted_hash: BlockHash,
        chain_id_hash: [u8; 32],
    ) -> Self {
        Self {
            trusted_validator_set,
            trusted_state: Mutex::new(TrustedState {
                height: trusted_height,
                hash: Some(trusted_hash),
            }),
            chain_id_hash,
        }
    }

    /// Verify a block header against the given QC and the trusted validator set.
    ///
    /// Checks:
    /// 1. QC's block_hash matches the header's hash
    /// 2. The QC has quorum (>= 2f+1 voting power)
    /// 3. The QC's aggregate signature is valid against the validator set
    pub fn verify_header(
        &self,
        header: &BlockHeader,
        qc: &QuorumCertificate,
        verifier: &dyn Verifier,
    ) -> Result<()> {
        let mut trusted = self.trusted_state()?;

        if header.height <= trusted.height {
            return Err(eg!(
                "header height {} <= trusted height {}",
                header.height.as_u64(),
                trusted.height.as_u64()
            ));
        }

        let trusted_hash = trusted.hash.ok_or_else(|| {
            eg!(
                "trusted hash unavailable at height {}; use new_with_trusted_hash or a verified trust path",
                trusted.height.as_u64()
            )
        })?;
        let expected_height = trusted
            .height
            .as_u64()
            .checked_add(1)
            .map(Height)
            .ok_or_else(|| eg!("trusted height overflow"))?;
        if header.height != expected_height {
            return Err(eg!(
                "non-adjacent header height {} does not extend trusted height {}; verify intermediate headers first",
                header.height.as_u64(),
                trusted.height.as_u64()
            ));
        }
        if header.parent_hash != trusted_hash {
            return Err(eg!(
                "header parent hash mismatch: expected {}, got {}",
                trusted_hash,
                header.parent_hash
            ));
        }

        // 1. Check QC's block_hash matches the header's hash
        if qc.block_hash != header.hash {
            return Err(eg!(
                "QC block_hash mismatch: expected {}, got {}",
                header.hash,
                qc.block_hash
            ));
        }

        // 2. Check quorum
        if !has_quorum(&self.trusted_validator_set, &qc.aggregate_signature) {
            return Err(eg!("QC does not have quorum"));
        }

        // 3. Verify each aggregate signature against the signer-specific vote payload.
        if qc.aggregate_signature.signers.len() != self.trusted_validator_set.validator_count() {
            return Err(eg!("QC signer bitfield length mismatch"));
        }
        let mut sig_idx = 0usize;
        for (idx, signed) in qc.aggregate_signature.signers.iter().enumerate() {
            if !signed {
                continue;
            }
            let Some(validator) = self.trusted_validator_set.validators().get(idx) else {
                return Err(eg!("QC signer index out of bounds"));
            };
            let Some(signature) = qc.aggregate_signature.signatures.get(sig_idx) else {
                return Err(eg!("QC signature count below signer bitfield"));
            };
            let signing_bytes = Vote::signing_bytes(
                &self.chain_id_hash,
                qc.epoch,
                qc.view,
                validator.id,
                &qc.block_hash,
                VoteType::Vote,
                None,
            );
            if !verifier.verify(&validator.public_key, &signing_bytes, signature) {
                return Err(eg!("QC aggregate signature verification failed"));
            }
            sig_idx += 1;
        }
        if sig_idx != qc.aggregate_signature.signatures.len() {
            return Err(eg!("QC aggregate signature verification failed"));
        }

        trusted.height = header.height;
        trusted.hash = Some(header.hash);
        Ok(())
    }

    /// Update the trusted validator set after an epoch transition.
    pub fn update_validator_set(
        &mut self,
        new_vs: ValidatorSet,
        new_height: Height,
        new_hash: BlockHash,
    ) {
        self.trusted_validator_set = new_vs;
        *self
            .trusted_state
            .get_mut()
            .expect("trusted state mutex poisoned") = TrustedState {
            height: new_height,
            hash: Some(new_hash),
        };
    }

    /// Return the current trusted height.
    pub fn trusted_height(&self) -> Height {
        self.trusted_state
            .lock()
            .expect("trusted state mutex poisoned")
            .height
    }

    /// Return the current trusted block hash, if this client has one.
    pub fn trusted_hash(&self) -> Option<BlockHash> {
        self.trusted_state
            .lock()
            .expect("trusted state mutex poisoned")
            .hash
    }

    /// Return a reference to the current trusted validator set.
    pub fn trusted_validator_set(&self) -> &ValidatorSet {
        &self.trusted_validator_set
    }

    fn trusted_state(&self) -> Result<MutexGuard<'_, TrustedState>> {
        self.trusted_state
            .lock()
            .map_err(|_| eg!("trusted state mutex poisoned"))
    }

    /// Verify an MPT state proof against a trusted app_hash.
    ///
    /// The `app_hash` should come from a verified block header (after
    /// `verify_header` succeeds). The `proof_bytes` are the serialized
    /// `MptProof` nodes (as returned by the `query` RPC `proof` field).
    /// The `expected_key` is the raw key the caller expects the proof to cover.
    ///
    /// Returns `Ok(true)` if the proof is valid against the given root.
    pub fn verify_state_proof(
        app_hash: &[u8; 32],
        expected_key: &[u8],
        proof: &vsdb::MptProof,
    ) -> ruc::Result<bool> {
        vsdb::MptCalc::verify_proof(app_hash, expected_key, proof)
            .map_err(|e| ruc::eg!(format!("MPT proof verification failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotmint_crypto::Ed25519Signer;
    use hotmint_crypto::Ed25519Verifier;
    use hotmint_crypto::aggregate::aggregate_votes;
    use hotmint_types::crypto::Signer;
    use hotmint_types::epoch::EpochNumber;
    use hotmint_types::validator::ValidatorInfo;

    const TEST_CHAIN: [u8; 32] = [0u8; 32];

    fn make_env() -> (ValidatorSet, Vec<Ed25519Signer>) {
        let signers: Vec<Ed25519Signer> = (0..4)
            .map(|i| Ed25519Signer::generate(ValidatorId(i)))
            .collect();
        let infos: Vec<ValidatorInfo> = signers
            .iter()
            .map(|s| ValidatorInfo {
                id: s.validator_id(),
                public_key: s.public_key(),
                power: 1,
            })
            .collect();
        (ValidatorSet::new(infos), signers)
    }

    fn make_header(height: u64, hash: BlockHash) -> BlockHeader {
        make_header_with_parent(height, BlockHash::GENESIS, hash)
    }

    fn make_header_with_parent(
        height: u64,
        parent_hash: BlockHash,
        hash: BlockHash,
    ) -> BlockHeader {
        BlockHeader {
            height: Height(height),
            parent_hash,
            view: ViewNumber(height),
            proposer: ValidatorId(0),
            timestamp: 0,
            app_hash: BlockHash::GENESIS,
            hash,
        }
    }

    fn make_qc(
        signers: &[Ed25519Signer],
        vs: &ValidatorSet,
        block_hash: BlockHash,
        view: ViewNumber,
        count: usize,
    ) -> QuorumCertificate {
        let epoch = EpochNumber(0);
        let votes: Vec<hotmint_types::vote::Vote> = signers
            .iter()
            .take(count)
            .map(|s| {
                let bytes = Vote::signing_bytes(
                    &TEST_CHAIN,
                    epoch,
                    view,
                    s.validator_id(),
                    &block_hash,
                    VoteType::Vote,
                    None,
                );
                hotmint_types::vote::Vote {
                    epoch,
                    block_hash,
                    view,
                    validator: s.validator_id(),
                    signature: s.sign(&bytes),
                    vote_type: VoteType::Vote,
                    extension: None,
                }
            })
            .collect();
        let agg = aggregate_votes(vs, &votes).unwrap();
        QuorumCertificate {
            block_hash,
            view,
            aggregate_signature: agg,
            epoch,
        }
    }

    #[test]
    fn test_valid_qc_passes_verification() {
        let (vs, signers) = make_env();
        let hash = BlockHash([1u8; 32]);
        let header = make_header(1, hash);
        let qc = make_qc(&signers, &vs, hash, ViewNumber(1), 3);
        let verifier = Ed25519Verifier;
        let client = LightClient::new(vs, Height(0), TEST_CHAIN);

        assert!(client.verify_header(&header, &qc, &verifier).is_ok());
        assert_eq!(client.trusted_height(), Height(1));
        assert_eq!(client.trusted_hash(), Some(hash));
    }

    #[test]
    fn test_wrong_block_hash_fails() {
        let (vs, signers) = make_env();
        let hash = BlockHash([1u8; 32]);
        let wrong_hash = BlockHash([2u8; 32]);
        let header = make_header(1, hash);
        // QC signs wrong_hash, but header has hash
        let qc = make_qc(&signers, &vs, wrong_hash, ViewNumber(1), 3);
        let verifier = Ed25519Verifier;
        let client = LightClient::new(vs, Height(0), TEST_CHAIN);

        let err = client.verify_header(&header, &qc, &verifier);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("block_hash mismatch"),
            "expected block_hash mismatch error"
        );
    }

    #[test]
    fn test_no_quorum_fails() {
        let (vs, signers) = make_env();
        let hash = BlockHash([1u8; 32]);
        let header = make_header(1, hash);
        // Only 2 out of 4 validators sign — below quorum threshold of 3
        let qc = make_qc(&signers, &vs, hash, ViewNumber(1), 2);
        let verifier = Ed25519Verifier;
        let client = LightClient::new(vs, Height(0), TEST_CHAIN);

        let err = client.verify_header(&header, &qc, &verifier);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("quorum"),
            "expected quorum error"
        );
        assert_eq!(client.trusted_height(), Height(0));
        assert_eq!(client.trusted_hash(), Some(BlockHash::GENESIS));
    }

    #[test]
    fn test_update_validator_set() {
        let (vs, _signers) = make_env();
        let mut client = LightClient::new(vs.clone(), Height(0), TEST_CHAIN);
        assert_eq!(client.trusted_height(), Height(0));

        let new_signers: Vec<Ed25519Signer> = (10..14)
            .map(|i| Ed25519Signer::generate(ValidatorId(i)))
            .collect();
        let new_infos: Vec<ValidatorInfo> = new_signers
            .iter()
            .map(|s| ValidatorInfo {
                id: s.validator_id(),
                public_key: s.public_key(),
                power: 1,
            })
            .collect();
        let new_vs = ValidatorSet::new(new_infos);

        let checkpoint_hash = BlockHash([9u8; 32]);
        client.update_validator_set(new_vs, Height(100), checkpoint_hash);
        assert_eq!(client.trusted_height(), Height(100));
        assert_eq!(client.trusted_hash(), Some(checkpoint_hash));
        assert_eq!(client.trusted_validator_set().validator_count(), 4);
    }

    #[test]
    fn test_parent_mismatch_fails_without_advancing_trust() {
        let (vs, signers) = make_env();
        let hash = BlockHash([1u8; 32]);
        let wrong_parent = BlockHash([9u8; 32]);
        let header = make_header_with_parent(1, wrong_parent, hash);
        let qc = make_qc(&signers, &vs, hash, ViewNumber(1), 3);
        let verifier = Ed25519Verifier;
        let client = LightClient::new(vs, Height(0), TEST_CHAIN);

        let err = client.verify_header(&header, &qc, &verifier);
        assert!(err.is_err());
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("parent hash mismatch"),
            "expected parent hash mismatch error"
        );
        assert_eq!(client.trusted_height(), Height(0));
        assert_eq!(client.trusted_hash(), Some(BlockHash::GENESIS));
    }

    #[test]
    fn test_non_adjacent_header_requires_verified_path() {
        let (vs, signers) = make_env();
        let hash = BlockHash([2u8; 32]);
        let header = make_header(2, hash);
        let qc = make_qc(&signers, &vs, hash, ViewNumber(2), 3);
        let verifier = Ed25519Verifier;
        let client = LightClient::new(vs, Height(0), TEST_CHAIN);

        let err = client.verify_header(&header, &qc, &verifier);
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("non-adjacent"),
            "expected non-adjacent path error"
        );
        assert_eq!(client.trusted_height(), Height(0));
    }

    #[test]
    fn test_trust_advances_along_hash_chain() {
        let (vs, signers) = make_env();
        let hash1 = BlockHash([1u8; 32]);
        let hash2 = BlockHash([2u8; 32]);
        let header1 = make_header(1, hash1);
        let header2 = make_header_with_parent(2, hash1, hash2);
        let qc1 = make_qc(&signers, &vs, hash1, ViewNumber(1), 3);
        let qc2 = make_qc(&signers, &vs, hash2, ViewNumber(2), 3);
        let verifier = Ed25519Verifier;
        let client = LightClient::new(vs, Height(0), TEST_CHAIN);

        client.verify_header(&header1, &qc1, &verifier).unwrap();
        client.verify_header(&header2, &qc2, &verifier).unwrap();

        assert_eq!(client.trusted_height(), Height(2));
        assert_eq!(client.trusted_hash(), Some(hash2));
    }
}
