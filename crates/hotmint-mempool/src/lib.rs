use std::cmp::Ordering;
use std::collections::HashSet;

use tokio::sync::Mutex;
use tracing::debug;

/// Transaction hash for deduplication
pub type TxHash = [u8; 32];

/// A transaction entry in the priority mempool.
#[derive(Clone)]
struct TxEntry {
    tx: Vec<u8>,
    priority: u64,
    hash: TxHash,
}

impl Eq for TxEntry {}

impl PartialEq for TxEntry {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

/// Order by (priority DESC, hash ASC) so the *last* element in BTreeSet
/// is the highest-priority tx and the *first* is the lowest.
impl Ord for TxEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.hash.cmp(&self.hash)) // reverse hash for tie-break
    }
}

impl PartialOrd for TxEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Priority-based mempool with deduplication and eviction.
///
/// Transactions are ordered by priority (highest first). When the pool is
/// full, a new transaction with higher priority than the lowest-priority
/// entry will evict it. This prevents spam DoS and enables fee-based
/// ordering for DeFi applications.
pub struct Mempool {
    entries: Mutex<std::collections::BTreeSet<TxEntry>>,
    seen: Mutex<HashSet<TxHash>>,
    max_size: usize,
    max_tx_bytes: usize,
}

impl Mempool {
    pub fn new(max_size: usize, max_tx_bytes: usize) -> Self {
        Self {
            entries: Mutex::new(std::collections::BTreeSet::new()),
            seen: Mutex::new(HashSet::new()),
            max_size,
            max_tx_bytes,
        }
    }

    /// Add a transaction with a given priority.
    ///
    /// Returns `true` if accepted. When the pool is full, the new tx is
    /// accepted only if its priority exceeds the lowest-priority entry,
    /// which is then evicted.
    pub async fn add_tx(&self, tx: Vec<u8>, priority: u64) -> bool {
        if tx.len() > self.max_tx_bytes {
            debug!(size = tx.len(), max = self.max_tx_bytes, "tx too large");
            return false;
        }

        let hash = Self::hash_tx(&tx);

        // Lock order: entries first, then seen
        let mut entries = self.entries.lock().await;
        let mut seen = self.seen.lock().await;

        if seen.contains(&hash) {
            return false;
        }

        if entries.len() >= self.max_size {
            // Check if new tx beats the lowest-priority entry
            if let Some(lowest) = entries.first() {
                if priority <= lowest.priority {
                    debug!(
                        priority,
                        lowest = lowest.priority,
                        "mempool full, priority too low"
                    );
                    return false;
                }
                // Evict lowest
                let evicted = entries.pop_first().expect("just checked non-empty");
                seen.remove(&evicted.hash);
                debug!(
                    evicted_priority = evicted.priority,
                    new_priority = priority,
                    "evicted low-priority tx"
                );
            }
        }

        seen.insert(hash);
        entries.insert(TxEntry { tx, priority, hash });
        true
    }

    /// Collect transactions for a block proposal (up to max_bytes total).
    /// Collected transactions are removed from the pool and the seen set.
    /// Transactions are collected in priority order (highest first).
    /// The payload is length-prefixed: `[u32_le len][bytes]...`
    pub async fn collect_payload(&self, max_bytes: usize) -> Vec<u8> {
        let mut entries = self.entries.lock().await;
        let mut seen = self.seen.lock().await;
        let mut payload = Vec::new();

        while let Some(entry) = entries.last() {
            // 4 bytes length prefix + tx bytes
            if payload.len() + 4 + entry.tx.len() > max_bytes {
                break;
            }
            let entry = entries.pop_last().expect("just checked non-empty");
            seen.remove(&entry.hash);
            let len = entry.tx.len() as u32;
            payload.extend_from_slice(&len.to_le_bytes());
            payload.extend_from_slice(&entry.tx);
        }

        payload
    }

    /// Reap collected payload back into individual transactions
    pub fn decode_payload(payload: &[u8]) -> Vec<Vec<u8>> {
        let mut txs = Vec::new();
        let mut offset = 0;
        while offset + 4 <= payload.len() {
            let len = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            if offset + len > payload.len() {
                break;
            }
            txs.push(payload[offset..offset + len].to_vec());
            offset += len;
        }
        txs
    }

    pub async fn size(&self) -> usize {
        self.entries.lock().await.len()
    }

    fn hash_tx(tx: &[u8]) -> TxHash {
        *blake3::hash(tx).as_bytes()
    }
}

impl Default for Mempool {
    fn default() -> Self {
        Self::new(10_000, 1_048_576) // 10k txs, 1MB max per tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_collect() {
        let pool = Mempool::new(100, 1024);
        assert!(pool.add_tx(b"tx1".to_vec(), 10).await);
        assert!(pool.add_tx(b"tx2".to_vec(), 20).await);
        assert_eq!(pool.size().await, 2);

        let payload = pool.collect_payload(1024).await;
        let txs = Mempool::decode_payload(&payload);
        assert_eq!(txs.len(), 2);
        // Higher priority first
        assert_eq!(txs[0], b"tx2");
        assert_eq!(txs[1], b"tx1");
    }

    #[tokio::test]
    async fn test_dedup() {
        let pool = Mempool::new(100, 1024);
        assert!(pool.add_tx(b"tx1".to_vec(), 10).await);
        assert!(!pool.add_tx(b"tx1".to_vec(), 10).await); // duplicate
        assert_eq!(pool.size().await, 1);
    }

    #[tokio::test]
    async fn test_eviction_by_priority() {
        let pool = Mempool::new(2, 1024);
        assert!(pool.add_tx(b"low".to_vec(), 1).await);
        assert!(pool.add_tx(b"mid".to_vec(), 5).await);
        // Pool full, but new tx has higher priority → evicts lowest
        assert!(pool.add_tx(b"high".to_vec(), 10).await);
        assert_eq!(pool.size().await, 2);

        let payload = pool.collect_payload(1024).await;
        let txs = Mempool::decode_payload(&payload);
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0], b"high");
        assert_eq!(txs[1], b"mid");
    }

    #[tokio::test]
    async fn test_reject_low_priority_when_full() {
        let pool = Mempool::new(2, 1024);
        assert!(pool.add_tx(b"a".to_vec(), 5).await);
        assert!(pool.add_tx(b"b".to_vec(), 10).await);
        // New tx has lower priority than lowest → rejected
        assert!(!pool.add_tx(b"c".to_vec(), 3).await);
        assert_eq!(pool.size().await, 2);
    }

    #[tokio::test]
    async fn test_tx_too_large() {
        let pool = Mempool::new(100, 4);
        assert!(!pool.add_tx(b"toolarge".to_vec(), 10).await);
        assert!(pool.add_tx(b"ok".to_vec(), 10).await);
    }

    #[tokio::test]
    async fn test_collect_respects_max_bytes() {
        let pool = Mempool::new(100, 1024);
        pool.add_tx(b"aaaa".to_vec(), 1).await;
        pool.add_tx(b"bbbb".to_vec(), 2).await;
        pool.add_tx(b"cccc".to_vec(), 3).await;

        // Each tx: 4 bytes len prefix + 4 bytes data = 8 bytes
        // max_bytes = 17 should fit 2 txs (16 bytes) but not 3 (24 bytes)
        let payload = pool.collect_payload(17).await;
        let txs = Mempool::decode_payload(&payload);
        assert_eq!(txs.len(), 2);
        // Highest priority first
        assert_eq!(txs[0], b"cccc");
        assert_eq!(txs[1], b"bbbb");
    }

    #[test]
    fn test_decode_empty_payload() {
        let txs = Mempool::decode_payload(&[]);
        assert!(txs.is_empty());
    }

    #[test]
    fn test_decode_truncated_payload() {
        // Only 2 bytes when expecting at least 4 for length prefix
        let txs = Mempool::decode_payload(&[1, 2]);
        assert!(txs.is_empty());
    }

    #[test]
    fn test_decode_payload_with_truncated_data() {
        // Length prefix says 100 bytes but only 3 available
        let mut payload = vec![];
        payload.extend_from_slice(&100u32.to_le_bytes());
        payload.extend_from_slice(&[1, 2, 3]);
        let txs = Mempool::decode_payload(&payload);
        assert!(txs.is_empty());
    }

    #[tokio::test]
    async fn test_empty_tx() {
        let pool = Mempool::new(100, 1024);
        assert!(pool.add_tx(vec![], 0).await);
        let payload = pool.collect_payload(1024).await;
        let txs = Mempool::decode_payload(&payload);
        assert_eq!(txs.len(), 1);
        assert!(txs[0].is_empty());
    }
}
