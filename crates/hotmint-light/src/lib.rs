//! Light client verification for Hotmint BFT consensus.
//!
//! Verifies block headers using QC signatures without downloading full blocks.
//! Also provides MPT state proof verification via [`LightClient::verify_state_proof`].

pub use vsdb::MptProof;

use ruc::*;

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
    trusted_height: Height,
    chain_id_hash: [u8; 32],
}

impl LightClient {
    /// Create a new light client with a trusted validator set and height.
    pub fn new(
        trusted_validator_set: ValidatorSet,
        trusted_height: Height,
        chain_id_hash: [u8; 32],
    ) -> Self {
        Self {
            trusted_validator_set,
            trusted_height,
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
        // A-6: Enforce height monotonicity — reject replayed or older headers.
        if header.height <= self.trusted_height {
            return Err(eg!(
                "header height {} <= trusted height {}",
                header.height.as_u64(),
                self.trusted_height.as_u64()
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

        // 3. Verify aggregate signature
        let signing_bytes = Vote::signing_bytes(
            &self.chain_id_hash,
            qc.epoch,
            qc.view,
            &qc.block_hash,
            VoteType::Vote,
        );
        if !verifier.verify_aggregate(
            &self.trusted_validator_set,
            &signing_bytes,
            &qc.aggregate_signature,
        ) {
            return Err(eg!("QC aggregate signature verification failed"));
        }

        Ok(())
    }

    /// Update the trusted validator set after an epoch transition.
    pub fn update_validator_set(&mut self, new_vs: ValidatorSet, new_height: Height) {
        self.trusted_validator_set = new_vs;
        self.trusted_height = new_height;
    }

    /// Return the current trusted height.
    pub fn trusted_height(&self) -> Height {
        self.trusted_height
    }

    /// Return a reference to the current trusted validator set.
    pub fn trusted_validator_set(&self) -> &ValidatorSet {
        &self.trusted_validator_set
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
        BlockHeader {
            height: Height(height),
            parent_hash: BlockHash::GENESIS,
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
                let bytes =
                    Vote::signing_bytes(&TEST_CHAIN, epoch, view, &block_hash, VoteType::Vote);
                hotmint_types::vote::Vote {
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

        client.update_validator_set(new_vs, Height(100));
        assert_eq!(client.trusted_height(), Height(100));
        assert_eq!(client.trusted_validator_set().validator_count(), 4);
    }
}
