use hotmint_consensus::store::BlockStore;
use hotmint_types::{Block, BlockHash, Height, QuorumCertificate};
use ruc::*;
use std::path::Path;
use tracing::debug;
use vsdb::MapxOrd;

/// File name for the persisted instance IDs of the block store collections.
const META_FILE: &str = "block_store.meta";

/// Persistent block store backed by vsdb
pub struct VsdbBlockStore {
    by_hash: MapxOrd<[u8; 32], Block>,
    by_height: MapxOrd<u64, [u8; 32]>,
    commit_qcs: MapxOrd<u64, QuorumCertificate>,
}

impl VsdbBlockStore {
    /// Opens an existing block store or creates a fresh one.
    ///
    /// Must be called after [`vsdb::vsdb_set_base_dir`].
    /// The instance IDs of the three internal collections are stored in
    /// `data_dir/block_store.meta` (24 bytes: three little-endian u64s).
    /// On first run the file is created; on subsequent runs the collections
    /// are recovered from their saved IDs via [`MapxOrd::from_meta`].
    pub fn open(data_dir: &Path) -> Result<Self> {
        let meta_path = data_dir.join(META_FILE);
        if meta_path.exists() {
            let bytes = std::fs::read(&meta_path).c(d!("read block_store.meta"))?;
            if bytes.len() != 24 {
                return Err(eg!(
                    "corrupt block_store.meta: expected 24 bytes, got {}",
                    bytes.len()
                ));
            }
            let by_hash_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
            let by_height_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
            let commit_qcs_id = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
            Ok(Self {
                by_hash: MapxOrd::from_meta(by_hash_id).c(d!("restore by_hash"))?,
                by_height: MapxOrd::from_meta(by_height_id).c(d!("restore by_height"))?,
                commit_qcs: MapxOrd::from_meta(commit_qcs_id).c(d!("restore commit_qcs"))?,
            })
        } else {
            let by_hash: MapxOrd<[u8; 32], Block> = MapxOrd::new();
            let by_height: MapxOrd<u64, [u8; 32]> = MapxOrd::new();
            let commit_qcs: MapxOrd<u64, QuorumCertificate> = MapxOrd::new();

            let by_hash_id = by_hash.save_meta().c(d!())?;
            let by_height_id = by_height.save_meta().c(d!())?;
            let commit_qcs_id = commit_qcs.save_meta().c(d!())?;

            let mut meta = [0u8; 24];
            meta[0..8].copy_from_slice(&by_hash_id.to_le_bytes());
            meta[8..16].copy_from_slice(&by_height_id.to_le_bytes());
            meta[16..24].copy_from_slice(&commit_qcs_id.to_le_bytes());
            std::fs::write(&meta_path, meta).c(d!("write block_store.meta"))?;

            let mut store = Self {
                by_hash,
                by_height,
                commit_qcs,
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
        self.by_height.insert(&block.height.as_u64(), &block.hash.0);
        self.by_hash.insert(&block.hash.0, &block);
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
        self.commit_qcs.insert(&height.as_u64(), &qc);
    }

    fn get_commit_qc(&self, height: Height) -> Option<QuorumCertificate> {
        self.commit_qcs.get(&height.as_u64())
    }

    fn flush(&self) {
        vsdb::vsdb_flush();
    }
}
