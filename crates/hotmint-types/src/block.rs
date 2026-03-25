use serde::{Deserialize, Serialize};
use std::fmt;

use crate::evidence::EquivocationProof;
use crate::validator::ValidatorId;
use crate::view::ViewNumber;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct BlockHash(pub [u8; 32]);

impl BlockHash {
    pub const GENESIS: Self = Self([0u8; 32]);

    pub fn is_genesis(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl fmt::Display for BlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0[..4]))
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Height(pub u64);

impl Height {
    pub const GENESIS: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("height overflow"))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Height {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "h{}", self.0)
    }
}

impl From<u64> for Height {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

/// Block B_k := (b_k, h_{k-1})
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub height: Height,
    pub parent_hash: BlockHash,
    pub view: ViewNumber,
    pub proposer: ValidatorId,
    /// Unix timestamp in milliseconds, set by the proposer.
    ///
    /// Validators verify that `timestamp >= parent.timestamp` and that
    /// it is within a reasonable drift window of the local clock.
    #[serde(default)]
    pub timestamp: u64,
    pub payload: Vec<u8>,
    /// Application state root after executing the **parent** block.
    pub app_hash: BlockHash,
    /// Equivocation evidence collected by the proposer (C-3).
    #[serde(default)]
    pub evidence: Vec<EquivocationProof>,
    pub hash: BlockHash,
}

impl Block {
    pub fn genesis() -> Self {
        Self {
            height: Height::GENESIS,
            parent_hash: BlockHash::GENESIS,
            view: ViewNumber::GENESIS,
            proposer: ValidatorId::default(),
            timestamp: 0,
            payload: Vec::new(),
            app_hash: BlockHash::GENESIS,
            evidence: Vec::new(),
            hash: BlockHash::GENESIS,
        }
    }

    /// Compute the Blake3 hash of this block's content and return it.
    pub fn compute_hash(&self) -> BlockHash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.height.as_u64().to_le_bytes());
        hasher.update(&self.parent_hash.0);
        hasher.update(&self.view.as_u64().to_le_bytes());
        hasher.update(&self.proposer.0.to_le_bytes());
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.app_hash.0);
        for ev in &self.evidence {
            hasher.update(&ev.validator.0.to_le_bytes());
            hasher.update(&ev.view.as_u64().to_le_bytes());
        }
        hasher.update(&self.payload);
        let hash = hasher.finalize();
        BlockHash(*hash.as_bytes())
    }
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_hash_genesis() {
        assert!(BlockHash::GENESIS.is_genesis());
        assert!(!BlockHash([1u8; 32]).is_genesis());
    }

    #[test]
    fn test_block_hash_display() {
        let h = BlockHash([
            0xab, 0xcd, 0xef, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0,
        ]);
        assert_eq!(format!("{h}"), "abcdef12");
    }

    #[test]
    fn test_height_next() {
        assert_eq!(Height(0).next(), Height(1));
        assert_eq!(Height(99).next(), Height(100));
    }

    #[test]
    fn test_height_ordering() {
        assert!(Height(1) < Height(2));
        assert!(Height(5) > Height(3));
        assert!(Height(0) <= Height::GENESIS);
    }

    #[test]
    fn test_genesis_block() {
        let g = Block::genesis();
        assert_eq!(g.height, Height::GENESIS);
        assert!(g.parent_hash.is_genesis());
        assert!(g.hash.is_genesis());
        assert!(g.payload.is_empty());
    }
}
