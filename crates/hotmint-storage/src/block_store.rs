use hotmint_consensus::store::BlockStore;
use hotmint_types::{Block, BlockHash, EndBlockResponse, Height, QuorumCertificate};
use ruc::*;
use std::path::Path;
use tracing::{debug, warn};
use vsdb::MapxOrd;

/// File name for the persisted instance IDs of the block store collections.
const META_FILE: &str = "block_store.meta";

/// Persistent block store backed by vsdb
pub struct VsdbBlockStore {
    by_hash: MapxOrd<[u8; 32], Block>,
    by_height: MapxOrd<u64, [u8; 32]>,
    commit_qcs: MapxOrd<u64, QuorumCertificate>,
    /// tx_hash → (height, tx_index_in_block)
    tx_index: MapxOrd<[u8; 32], (u64, u32)>,
    /// height → EndBlockResponse (block execution results)
    block_results: MapxOrd<u64, EndBlockResponse>,
}

impl VsdbBlockStore {
    /// Opens an existing block store or creates a fresh one.
    ///
    /// Must be called after [`vsdb::vsdb_set_base_dir`].
    /// The instance IDs of the internal collections are stored in
    /// `data_dir/block_store.meta`. On first run the file is created;
    /// on subsequent runs the collections are recovered from saved IDs.
    ///
    /// Backward-compatible: 24-byte meta (v1, 3 collections) is auto-migrated
    /// to 40 bytes (v2, 5 collections) on first open.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let meta_path = data_dir.join(META_FILE);
        if meta_path.exists() {
            let bytes = std::fs::read(&meta_path).c(d!("read block_store.meta"))?;
            if bytes.len() == 24 {
                // v1 meta: migrate by creating two new collections.
                let by_hash_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let by_height_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                let commit_qcs_id = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
                let tx_index: MapxOrd<[u8; 32], (u64, u32)> = MapxOrd::new();
                let block_results: MapxOrd<u64, EndBlockResponse> = MapxOrd::new();
                let tx_index_id = tx_index.save_meta().c(d!())?;
                let block_results_id = block_results.save_meta().c(d!())?;
                let mut meta = [0u8; 40];
                meta[0..8].copy_from_slice(&by_hash_id.to_le_bytes());
                meta[8..16].copy_from_slice(&by_height_id.to_le_bytes());
                meta[16..24].copy_from_slice(&commit_qcs_id.to_le_bytes());
                meta[24..32].copy_from_slice(&tx_index_id.to_le_bytes());
                meta[32..40].copy_from_slice(&block_results_id.to_le_bytes());
                {
                    use std::io::Write;
                    let mut f =
                        std::fs::File::create(&meta_path).c(d!("create block_store.meta v2"))?;
                    f.write_all(&meta).c(d!("write block_store.meta v2"))?;
                    f.sync_all().c(d!("fsync block_store.meta v2"))?;
                }
                Ok(Self {
                    by_hash: MapxOrd::from_meta(by_hash_id).c(d!("restore by_hash"))?,
                    by_height: MapxOrd::from_meta(by_height_id).c(d!("restore by_height"))?,
                    commit_qcs: MapxOrd::from_meta(commit_qcs_id).c(d!("restore commit_qcs"))?,
                    tx_index,
                    block_results,
                })
            } else if bytes.len() == 40 {
                let by_hash_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let by_height_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                let commit_qcs_id = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
                let tx_index_id = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
                let block_results_id = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
                Ok(Self {
                    by_hash: MapxOrd::from_meta(by_hash_id).c(d!("restore by_hash"))?,
                    by_height: MapxOrd::from_meta(by_height_id).c(d!("restore by_height"))?,
                    commit_qcs: MapxOrd::from_meta(commit_qcs_id).c(d!("restore commit_qcs"))?,
                    tx_index: MapxOrd::from_meta(tx_index_id).c(d!("restore tx_index"))?,
                    block_results: MapxOrd::from_meta(block_results_id)
                        .c(d!("restore block_results"))?,
                })
            } else {
                Err(eg!(
                    "corrupt block_store.meta: expected 24 or 40 bytes, got {}",
                    bytes.len()
                ))
            }
        } else {
            let by_hash: MapxOrd<[u8; 32], Block> = MapxOrd::new();
            let by_height: MapxOrd<u64, [u8; 32]> = MapxOrd::new();
            let commit_qcs: MapxOrd<u64, QuorumCertificate> = MapxOrd::new();
            let tx_index: MapxOrd<[u8; 32], (u64, u32)> = MapxOrd::new();
            let block_results: MapxOrd<u64, EndBlockResponse> = MapxOrd::new();

            let by_hash_id = by_hash.save_meta().c(d!())?;
            let by_height_id = by_height.save_meta().c(d!())?;
            let commit_qcs_id = commit_qcs.save_meta().c(d!())?;
            let tx_index_id = tx_index.save_meta().c(d!())?;
            let block_results_id = block_results.save_meta().c(d!())?;

            let mut meta = [0u8; 40];
            meta[0..8].copy_from_slice(&by_hash_id.to_le_bytes());
            meta[8..16].copy_from_slice(&by_height_id.to_le_bytes());
            meta[16..24].copy_from_slice(&commit_qcs_id.to_le_bytes());
            meta[24..32].copy_from_slice(&tx_index_id.to_le_bytes());
            meta[32..40].copy_from_slice(&block_results_id.to_le_bytes());
            {
                use std::io::Write;
                let mut f = std::fs::File::create(&meta_path).c(d!("create block_store.meta"))?;
                f.write_all(&meta).c(d!("write block_store.meta"))?;
                f.sync_all().c(d!("fsync block_store.meta"))?;
            }

            let mut store = Self {
                by_hash,
                by_height,
                commit_qcs,
                tx_index,
                block_results,
            };
            store.put_block(Block::genesis());
            Ok(store)
        }
    }

    /// Creates a new in-memory block store without any persistent meta file.
    /// Intended for unit tests only; use [`Self::open`] in production.
    pub fn new() -> Self {
        let mut store = Self {
            by_hash: MapxOrd::new(),
            by_height: MapxOrd::new(),
            commit_qcs: MapxOrd::new(),
            tx_index: MapxOrd::new(),
            block_results: MapxOrd::new(),
        };
        store.put_block(Block::genesis());
        store
    }

    pub fn contains(&self, hash: &BlockHash) -> bool {
        self.by_hash.contains_key(&hash.0)
    }

    pub fn flush(&self) {
        vsdb::vsdb_flush();
    }
}

impl Default for VsdbBlockStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockStore for VsdbBlockStore {
    fn put_block(&mut self, block: Block) {
        debug!(height = block.height.as_u64(), hash = %block.hash, "storing block to vsdb");
        // Insert by_hash first so a crash between the two inserts leaves the
        // block data present (recoverable) rather than a dangling height index.
        self.by_hash.insert(&block.hash.0, &block);
        let height = block.height.as_u64();
        if let Some(commit_qc) = self.commit_qcs.get(&height) {
            if commit_qc.block_hash == block.hash {
                self.by_height.insert(&height, &block.hash.0);
            }
        } else {
            self.by_height.insert(&height, &block.hash.0);
        }
    }

    fn get_block(&self, hash: &BlockHash) -> Option<Block> {
        self.by_hash.get(&hash.0)
    }

    fn get_block_by_height(&self, h: Height) -> Option<Block> {
        self.by_height
            .get(&h.as_u64())
            .and_then(|hash_bytes| self.by_hash.get(&hash_bytes))
    }

    fn get_blocks_in_range(&self, from: Height, to: Height) -> Vec<Block> {
        self.by_height
            .range(from.as_u64()..=to.as_u64())
            .filter_map(|(_, hash_bytes)| self.by_hash.get(&hash_bytes))
            .collect()
    }

    fn tip_height(&self) -> Height {
        self.by_height
            .last()
            .map(|(h, _)| Height(h))
            .unwrap_or(Height::GENESIS)
    }

    fn put_commit_qc(&mut self, height: Height, qc: QuorumCertificate) {
        let h = height.as_u64();
        self.commit_qcs.insert(&h, &qc);
        if let Some(block) = self.by_hash.get(&qc.block_hash.0) {
            if block.height == height {
                self.by_height.insert(&h, &qc.block_hash.0);
            } else {
                warn!(
                    qc_hash = %qc.block_hash,
                    qc_height = h,
                    block_height = block.height.as_u64(),
                    "commit QC height does not match stored block height; not repinning height index"
                );
            }
        }
    }

    fn get_commit_qc(&self, height: Height) -> Option<QuorumCertificate> {
        self.commit_qcs.get(&height.as_u64())
    }

    fn flush(&self) {
        vsdb::vsdb_flush();
    }

    fn put_tx_index(&mut self, tx_hash: [u8; 32], height: Height, index: u32) {
        self.tx_index.insert(&tx_hash, &(height.as_u64(), index));
    }

    fn get_tx_location(&self, tx_hash: &[u8; 32]) -> Option<(Height, u32)> {
        self.tx_index.get(tx_hash).map(|(h, idx)| (Height(h), idx))
    }

    fn put_block_results(&mut self, height: Height, results: EndBlockResponse) {
        self.block_results.insert(&height.as_u64(), &results);
    }

    fn get_block_results(&self, height: Height) -> Option<EndBlockResponse> {
        self.block_results.get(&height.as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotmint_types::{AggregateSignature, EpochNumber, ValidatorId, ViewNumber};

    fn make_block(height: u64, parent: BlockHash, byte: u8) -> Block {
        Block {
            height: Height(height),
            parent_hash: parent,
            view: ViewNumber(height),
            proposer: ValidatorId(0),
            timestamp: 0,
            payload: vec![],
            app_hash: BlockHash::GENESIS,
            evidence: Vec::new(),
            hash: BlockHash([byte; 32]),
        }
    }

    fn make_qc(block: &Block) -> QuorumCertificate {
        QuorumCertificate {
            block_hash: block.hash,
            view: block.view,
            aggregate_signature: AggregateSignature::new(1),
            epoch: EpochNumber(0),
        }
    }

    #[test]
    fn committed_height_index_repins_and_stays_pinned() {
        let mut store = VsdbBlockStore::new();
        let committed = make_block(1, BlockHash::GENESIS, 1);
        let proposal = make_block(1, BlockHash::GENESIS, 42);
        let later_proposal = make_block(1, BlockHash::GENESIS, 99);
        let later_hash = later_proposal.hash;

        store.put_block(committed.clone());
        store.put_block(proposal.clone());
        assert_eq!(
            store.get_block_by_height(Height(1)).map(|b| b.hash),
            Some(proposal.hash)
        );

        store.put_commit_qc(Height(1), make_qc(&committed));
        assert_eq!(
            store.get_block_by_height(Height(1)).map(|b| b.hash),
            Some(committed.hash)
        );

        store.put_block(later_proposal);
        assert_eq!(
            store.get_block_by_height(Height(1)).map(|b| b.hash),
            Some(committed.hash)
        );
        assert!(store.get_block(&later_hash).is_some());
        assert_eq!(
            store.get_commit_qc(Height(1)).map(|qc| qc.block_hash),
            Some(committed.hash)
        );
    }
}
