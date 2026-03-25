use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::RwLock;

use hotmint_types::{Block, BlockHash, Height, QuorumCertificate};

pub trait BlockStore: Send + Sync {
    fn put_block(&mut self, block: Block);
    fn get_block(&self, hash: &BlockHash) -> Option<Block>;
    fn get_block_by_height(&self, h: Height) -> Option<Block>;

    /// Store the QC that committed a block at the given height.
    fn put_commit_qc(&mut self, _height: Height, _qc: QuorumCertificate) {}
    /// Retrieve the commit QC for a block at the given height.
    fn get_commit_qc(&self, _height: Height) -> Option<QuorumCertificate> {
        None
    }

    /// Flush pending writes to durable storage.
    fn flush(&self) {}

    /// Get blocks in [from, to] inclusive. Default iterates one-by-one.
    fn get_blocks_in_range(&self, from: Height, to: Height) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut h = from.as_u64();
        while h <= to.as_u64() {
            if let Some(block) = self.get_block_by_height(Height(h)) {
                blocks.push(block);
            }
            h += 1;
        }
        blocks
    }

    /// Return the highest stored block height.
    fn tip_height(&self) -> Height {
        Height::GENESIS
    }

    /// Store a tx hash → (height, index) mapping.
    fn put_tx_index(&mut self, _tx_hash: [u8; 32], _height: Height, _index: u32) {}

    /// Look up a tx hash → (height, index_in_block).
    fn get_tx_location(&self, _tx_hash: &[u8; 32]) -> Option<(Height, u32)> {
        None
    }

    /// Store the EndBlockResponse for a given height.
    fn put_block_results(&mut self, _height: Height, _results: hotmint_types::EndBlockResponse) {}

    /// Retrieve the EndBlockResponse for a given height.
    fn get_block_results(&self, _height: Height) -> Option<hotmint_types::EndBlockResponse> {
        None
    }
}

/// Adapter that implements `BlockStore` over a shared `Arc<RwLock<Box<dyn BlockStore>>>`,
/// acquiring and releasing the lock for each individual operation.
///
/// Uses `read` / `write` on `parking_lot::RwLock` (R-1) for fast, non-poisoning
/// synchronous locking. The lock hold times are microsecond-level HashMap lookups.
pub struct SharedStoreAdapter(pub Arc<RwLock<Box<dyn BlockStore>>>);

impl BlockStore for SharedStoreAdapter {
    fn put_block(&mut self, block: Block) {
        self.0.write().put_block(block);
    }
    fn get_block(&self, hash: &BlockHash) -> Option<Block> {
        self.0.read().get_block(hash)
    }
    fn get_block_by_height(&self, h: Height) -> Option<Block> {
        self.0.read().get_block_by_height(h)
    }
    fn get_blocks_in_range(&self, from: Height, to: Height) -> Vec<Block> {
        self.0.read().get_blocks_in_range(from, to)
    }
    fn tip_height(&self) -> Height {
        self.0.read().tip_height()
    }
    fn put_commit_qc(&mut self, height: Height, qc: QuorumCertificate) {
        self.0.write().put_commit_qc(height, qc);
    }
    fn get_commit_qc(&self, height: Height) -> Option<QuorumCertificate> {
        self.0.read().get_commit_qc(height)
    }
    fn flush(&self) {
        self.0.read().flush();
    }
    fn put_tx_index(&mut self, tx_hash: [u8; 32], height: Height, index: u32) {
        self.0.write().put_tx_index(tx_hash, height, index);
    }
    fn get_tx_location(&self, tx_hash: &[u8; 32]) -> Option<(Height, u32)> {
        self.0.read().get_tx_location(tx_hash)
    }
    fn put_block_results(&mut self, height: Height, results: hotmint_types::EndBlockResponse) {
        self.0.write().put_block_results(height, results);
    }
    fn get_block_results(&self, height: Height) -> Option<hotmint_types::EndBlockResponse> {
        self.0.read().get_block_results(height)
    }
}

/// In-memory block store stub
pub struct MemoryBlockStore {
    by_hash: HashMap<BlockHash, Block>,
    by_height: BTreeMap<u64, BlockHash>,
    commit_qcs: HashMap<u64, QuorumCertificate>,
}

impl Default for MemoryBlockStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBlockStore {
    pub fn new() -> Self {
        let mut store = Self {
            by_hash: HashMap::new(),
            by_height: BTreeMap::new(),
            commit_qcs: HashMap::new(),
        };
        let genesis = Block::genesis();
        store.put_block(genesis);
        store
    }

    /// Create a new in-memory block store wrapped in `Arc<parking_lot::RwLock<Box<dyn BlockStore>>>`,
    /// ready for use with `ConsensusEngine`.
    pub fn new_shared() -> crate::engine::SharedBlockStore {
        Arc::new(RwLock::new(Box::new(Self::new())))
    }
}

impl BlockStore for MemoryBlockStore {
    fn put_block(&mut self, block: Block) {
        let hash = block.hash;
        self.by_height.insert(block.height.as_u64(), hash);
        self.by_hash.insert(hash, block);
    }

    fn get_block(&self, hash: &BlockHash) -> Option<Block> {
        self.by_hash.get(hash).cloned()
    }

    fn get_block_by_height(&self, h: Height) -> Option<Block> {
        self.by_height
            .get(&h.as_u64())
            .and_then(|hash| self.by_hash.get(hash))
            .cloned()
    }

    fn get_blocks_in_range(&self, from: Height, to: Height) -> Vec<Block> {
        self.by_height
            .range(from.as_u64()..=to.as_u64())
            .filter_map(|(_, hash)| self.by_hash.get(hash).cloned())
            .collect()
    }

    fn tip_height(&self) -> Height {
        self.by_height
            .keys()
            .next_back()
            .map(|h| Height(*h))
            .unwrap_or(Height::GENESIS)
    }

    fn put_commit_qc(&mut self, height: Height, qc: QuorumCertificate) {
        self.commit_qcs.insert(height.as_u64(), qc);
    }

    fn get_commit_qc(&self, height: Height) -> Option<QuorumCertificate> {
        self.commit_qcs.get(&height.as_u64()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotmint_types::{ValidatorId, ViewNumber};

    fn make_block(height: u64, parent: BlockHash) -> Block {
        let hash = BlockHash([height as u8; 32]);
        Block {
            height: Height(height),
            parent_hash: parent,
            view: ViewNumber(height),
            proposer: ValidatorId(0),
            timestamp: 0,
            payload: vec![],
            app_hash: BlockHash::GENESIS,
            evidence: Vec::new(),
            hash,
        }
    }

    #[test]
    fn test_genesis_present() {
        let store = MemoryBlockStore::new();
        let genesis = store.get_block(&BlockHash::GENESIS);
        assert!(genesis.is_some());
        assert_eq!(genesis.unwrap().height, Height::GENESIS);
    }

    #[test]
    fn test_put_and_get_by_hash() {
        let mut store = MemoryBlockStore::new();
        let block = make_block(1, BlockHash::GENESIS);
        let hash = block.hash;
        store.put_block(block);
        let retrieved = store.get_block(&hash).unwrap();
        assert_eq!(retrieved.height, Height(1));
    }

    #[test]
    fn test_get_by_height() {
        let mut store = MemoryBlockStore::new();
        let b1 = make_block(1, BlockHash::GENESIS);
        let b2 = make_block(2, b1.hash);
        store.put_block(b1);
        store.put_block(b2);

        assert!(store.get_block_by_height(Height(1)).is_some());
        assert!(store.get_block_by_height(Height(2)).is_some());
        assert!(store.get_block_by_height(Height(99)).is_none());
    }

    #[test]
    fn test_get_nonexistent() {
        let store = MemoryBlockStore::new();
        assert!(store.get_block(&BlockHash([99u8; 32])).is_none());
        assert!(store.get_block_by_height(Height(999)).is_none());
    }

    #[test]
    fn test_get_blocks_in_range() {
        let mut store = MemoryBlockStore::new();
        let b1 = make_block(1, BlockHash::GENESIS);
        let b2 = make_block(2, b1.hash);
        let b3 = make_block(3, b2.hash);
        store.put_block(b1);
        store.put_block(b2);
        store.put_block(b3);

        let blocks = store.get_blocks_in_range(Height(1), Height(3));
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].height, Height(1));
        assert_eq!(blocks[2].height, Height(3));

        // Partial range
        let blocks = store.get_blocks_in_range(Height(2), Height(3));
        assert_eq!(blocks.len(), 2);

        // Out of range
        let blocks = store.get_blocks_in_range(Height(10), Height(20));
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_tip_height() {
        let store = MemoryBlockStore::new();
        assert_eq!(store.tip_height(), Height::GENESIS);

        let mut store = MemoryBlockStore::new();
        let b1 = make_block(1, BlockHash::GENESIS);
        let b2 = make_block(2, b1.hash);
        store.put_block(b1);
        store.put_block(b2);
        assert_eq!(store.tip_height(), Height(2));
    }

    #[test]
    fn test_overwrite_same_height() {
        let mut store = MemoryBlockStore::new();
        let b1 = make_block(1, BlockHash::GENESIS);
        store.put_block(b1);
        // Different block at same height (different hash)
        let mut b2 = make_block(1, BlockHash::GENESIS);
        b2.hash = BlockHash([42u8; 32]);
        b2.payload = vec![1, 2, 3];
        store.put_block(b2);
        // Height now points to new block
        let retrieved = store.get_block_by_height(Height(1)).unwrap();
        assert_eq!(retrieved.hash, BlockHash([42u8; 32]));
    }
}
