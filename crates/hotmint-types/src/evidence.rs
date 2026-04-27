use serde::{Deserialize, Serialize};

use crate::block::BlockHash;
use crate::crypto::Signature;
use crate::validator::ValidatorId;
use crate::view::ViewNumber;
use crate::vote::VoteType;

/// Proof that a validator voted for two different blocks in the same (view, vote_type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivocationProof {
    pub validator: ValidatorId,
    pub view: ViewNumber,
    pub vote_type: VoteType,
    /// Epoch in which the equivocation occurred. Required for cross-epoch
    /// evidence verification (the signing_bytes include epoch).
    #[serde(default)]
    pub epoch: crate::epoch::EpochNumber,
    pub block_hash_a: BlockHash,
    pub signature_a: Signature,
    #[serde(default)]
    pub extension_a: Option<Vec<u8>>,
    pub block_hash_b: BlockHash,
    pub signature_b: Signature,
    #[serde(default)]
    pub extension_b: Option<Vec<u8>>,
}
