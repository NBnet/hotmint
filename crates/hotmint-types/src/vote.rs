use serde::{Deserialize, Serialize};

use crate::block::BlockHash;
use crate::crypto::Signature;
use crate::epoch::EpochNumber;
use crate::validator::ValidatorId;
use crate::view::ViewNumber;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VoteType {
    /// First-phase vote (step 3)
    Vote,
    /// Second-phase vote (step 5)
    Vote2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    /// Epoch in which this vote was signed.
    ///
    /// This is part of the signed payload and is used to select the validator
    /// set for both verification and QC formation.
    #[serde(default)]
    pub epoch: EpochNumber,
    pub block_hash: BlockHash,
    pub view: ViewNumber,
    pub validator: ValidatorId,
    pub signature: Signature,
    pub vote_type: VoteType,
    /// Optional vote extension data (ABCI++ Vote Extensions).
    /// Only meaningful for Vote2 (second-phase votes).
    #[serde(default)]
    pub extension: Option<Vec<u8>>,
}

impl Vote {
    /// Canonical bytes for signing:
    /// domain_tag || chain_id_hash || epoch || view || validator_id ||
    /// block_hash || vote_type || extension_marker || extension_len || extension_hash.
    ///
    /// The domain tag, chain_id_hash, epoch, validator_id, vote type, and
    /// extension digest prevent cross-chain, cross-epoch, cross-validator,
    /// cross-message-type, and Vote2-extension replay attacks.
    pub fn signing_bytes(
        chain_id_hash: &[u8; 32],
        epoch: EpochNumber,
        view: ViewNumber,
        validator: ValidatorId,
        block_hash: &BlockHash,
        vote_type: VoteType,
        extension: Option<&[u8]>,
    ) -> Vec<u8> {
        let tag = b"HOTMINT_VOTE_V3\0";
        let mut buf = Vec::with_capacity(tag.len() + 32 + 8 + 8 + 8 + 32 + 1 + 1 + 8 + 32);
        buf.extend_from_slice(tag);
        buf.extend_from_slice(chain_id_hash);
        buf.extend_from_slice(&epoch.as_u64().to_le_bytes());
        buf.extend_from_slice(&view.as_u64().to_le_bytes());
        buf.extend_from_slice(&validator.0.to_le_bytes());
        buf.extend_from_slice(&block_hash.0);
        buf.push(vote_type as u8);
        match extension {
            Some(bytes) => {
                buf.push(1);
                buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
                buf.extend_from_slice(blake3::hash(bytes).as_bytes());
            }
            None => {
                buf.push(0);
                buf.extend_from_slice(&0u64.to_le_bytes());
                buf.extend_from_slice(&[0u8; 32]);
            }
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epoch::EpochNumber;

    const TEST_CHAIN: [u8; 32] = [0u8; 32];

    #[test]
    fn test_signing_bytes_deterministic() {
        let hash = BlockHash([42u8; 32]);
        let a = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(5),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        let b = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(5),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn test_signing_bytes_differ_by_type() {
        let hash = BlockHash([1u8; 32]);
        let a = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        let b = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote2,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn test_signing_bytes_differ_by_view() {
        let hash = BlockHash([1u8; 32]);
        let a = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        let b = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(2),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn test_signing_bytes_differ_by_chain() {
        let hash = BlockHash([1u8; 32]);
        let chain_a = [1u8; 32];
        let chain_b = [2u8; 32];
        let a = Vote::signing_bytes(
            &chain_a,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        let b = Vote::signing_bytes(
            &chain_b,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn test_signing_bytes_differ_by_epoch() {
        let hash = BlockHash([1u8; 32]);
        let a = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        let b = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(1),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn test_signing_bytes_differ_by_validator() {
        let hash = BlockHash([1u8; 32]);
        let a = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote,
            None,
        );
        let b = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(1),
            &hash,
            VoteType::Vote,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn test_signing_bytes_differ_by_extension() {
        let hash = BlockHash([1u8; 32]);
        let a = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote2,
            Some(b"extension-a"),
        );
        let b = Vote::signing_bytes(
            &TEST_CHAIN,
            EpochNumber(0),
            ViewNumber(1),
            ValidatorId(0),
            &hash,
            VoteType::Vote2,
            Some(b"extension-b"),
        );
        assert_ne!(a, b);
    }
}
