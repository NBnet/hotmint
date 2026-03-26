//! Ethereum-compatible transaction pool with (sender, nonce) indexing.
//!
//! Provides:
//! - Pending/queued separation based on nonce continuity
//! - Replacement-by-fee with configurable price increase threshold
//! - Nonce-continuous payload selection ordered by effective tip
//! - Integration with the Application trait via create_payload

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use alloy_consensus::Transaction;
use alloy_primitives::{Address, B256};
use hotmint_evm_types::tx::{self, VerifiedTx};
use tracing::debug;

/// Configuration for the EVM transaction pool.
#[derive(Debug, Clone)]
pub struct EvmTxPoolConfig {
    /// Maximum number of pending transactions per sender.
    pub max_pending_per_sender: usize,
    /// Maximum number of queued transactions per sender.
    pub max_queued_per_sender: usize,
    /// Minimum gas price increase percentage for replacement (default: 10%).
    pub replacement_bump_pct: u64,
    /// Chain base fee for tip calculation.
    pub base_fee: u64,
}

impl Default for EvmTxPoolConfig {
    fn default() -> Self {
        Self {
            max_pending_per_sender: 64,
            max_queued_per_sender: 64,
            replacement_bump_pct: 10,
            base_fee: 1_000_000_000,
        }
    }
}

/// A transaction entry in the pool.
#[derive(Debug, Clone)]
struct PoolEntry {
    verified: VerifiedTx,
    effective_tip: u64,
}

/// Per-sender transaction queue.
#[derive(Debug, Default)]
struct SenderQueue {
    /// Pending: nonce-continuous from the current account nonce.
    /// Key = nonce.
    pending: BTreeMap<u64, PoolEntry>,
    /// Queued: nonce gaps exist, waiting for predecessors.
    queued: BTreeMap<u64, PoolEntry>,
}

/// Ethereum-compatible transaction pool.
pub struct EvmTxPool {
    config: EvmTxPoolConfig,
    inner: Mutex<PoolInner>,
}

struct PoolInner {
    /// Per-sender queues.
    senders: HashMap<Address, SenderQueue>,
    /// Tx hash → sender for quick lookup.
    by_hash: HashMap<B256, Address>,
    /// Function to get the current nonce for a sender.
    /// This is set by the executor when wiring up the pool.
    nonce_fn: Option<Box<dyn Fn(&Address) -> u64 + Send + Sync>>,
}

impl EvmTxPool {
    /// Create a new transaction pool.
    pub fn new(config: EvmTxPoolConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(PoolInner {
                senders: HashMap::new(),
                by_hash: HashMap::new(),
                nonce_fn: None,
            }),
        }
    }

    /// Set the nonce lookup function (called during pool operations).
    pub fn set_nonce_fn(&self, f: Box<dyn Fn(&Address) -> u64 + Send + Sync>) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .nonce_fn = Some(f);
    }

    /// Submit a raw Ethereum transaction to the pool.
    /// Returns the tx hash on success.
    pub fn submit_tx(&self, raw: &[u8]) -> Result<B256, String> {
        let verified = tx::decode_and_recover(raw).map_err(|e| e.to_string())?;
        let tx_hash = verified.tx_hash;

        let effective_tip = tx::effective_gas_tip(&verified.envelope, self.config.base_fee);
        let entry = PoolEntry {
            verified,
            effective_tip,
        };

        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // Check if tx hash already exists.
        if inner.by_hash.contains_key(&tx_hash) {
            return Err("transaction already known".to_string());
        }

        let sender = entry.verified.sender;
        let tx_nonce = entry.verified.envelope.nonce();
        let current_nonce = inner.nonce_fn.as_ref().map(|f| f(&sender)).unwrap_or(0);

        if tx_nonce < current_nonce {
            return Err(format!(
                "nonce too low: tx nonce {tx_nonce} < current {current_nonce}"
            ));
        }

        let queue = inner.senders.entry(sender).or_default();

        // Determine whether this goes to pending or queued.
        let is_pending = tx_nonce == current_nonce
            || queue
                .pending
                .contains_key(&tx_nonce.checked_sub(1).unwrap_or(0));

        let target = if is_pending {
            &mut queue.pending
        } else {
            &mut queue.queued
        };

        // Check for replacement.
        let mut old_hash = None;
        if let Some(existing) = target.get(&tx_nonce) {
            let min_tip = existing
                .effective_tip
                .saturating_mul(100 + self.config.replacement_bump_pct)
                / 100;
            if entry.effective_tip < min_tip {
                return Err(format!(
                    "replacement underpriced: need tip >= {min_tip}, got {}",
                    entry.effective_tip
                ));
            }
            old_hash = Some(existing.verified.tx_hash);
        }

        // Check capacity.
        let limit = if is_pending {
            self.config.max_pending_per_sender
        } else {
            self.config.max_queued_per_sender
        };

        if old_hash.is_none() && target.len() >= limit {
            return Err("sender queue full".to_string());
        }

        target.insert(tx_nonce, entry);

        // Clean up old hash and register new one.
        if let Some(h) = old_hash {
            inner.by_hash.remove(&h);
        }
        inner.by_hash.insert(tx_hash, sender);

        // Promote queued → pending if nonce gap is filled.
        let queue = inner.senders.get_mut(&sender).unwrap();
        Self::promote_queued_static(queue, current_nonce);

        debug!(sender = %sender, nonce = tx_nonce, "tx added to pool");
        Ok(tx_hash)
    }

    /// Collect transactions for a block proposal.
    ///
    /// Returns raw tx bytes in priority order (highest effective tip first),
    /// ensuring nonce continuity per sender.
    /// Returns length-prefixed format compatible with `decode_payload`.
    pub fn collect_payload(&self, max_bytes: usize, max_gas: u64) -> Vec<u8> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut payload = Vec::new();
        let mut total_gas = 0u64;

        // Collect the best pending tx from each sender, sorted by tip.
        let mut candidates: Vec<(Address, u64, u64, Vec<u8>)> = Vec::new(); // (sender, nonce, tip, raw)
        for (sender, queue) in &inner.senders {
            if let Some((&nonce, entry)) = queue.pending.iter().next() {
                candidates.push((
                    *sender,
                    nonce,
                    entry.effective_tip,
                    entry.verified.raw.clone(),
                ));
            }
        }
        // Sort by tip descending.
        candidates.sort_by(|a, b| b.2.cmp(&a.2));

        let mut included: Vec<(Address, u64)> = Vec::new();
        for (sender, nonce, _tip, raw) in &candidates {
            let gas = inner
                .senders
                .get(sender)
                .and_then(|q| q.pending.get(nonce))
                .map(|e| e.verified.envelope.gas_limit())
                .unwrap_or(0);

            if payload.len() + 4 + raw.len() > max_bytes {
                break;
            }
            if max_gas > 0 && total_gas + gas > max_gas {
                continue;
            }

            // Length-prefix encoding.
            payload.extend_from_slice(&(raw.len() as u32).to_le_bytes());
            payload.extend_from_slice(raw);
            total_gas += gas;
            included.push((*sender, *nonce));
        }

        // Remove included transactions.
        for (sender, nonce) in &included {
            let hash = inner
                .senders
                .get_mut(sender)
                .and_then(|q| q.pending.remove(nonce))
                .map(|e| e.verified.tx_hash);
            if let Some(h) = hash {
                inner.by_hash.remove(&h);
            }
        }

        payload
    }

    /// Remove committed transactions and promote queued ones.
    pub fn on_commit(&self, nonce_fn: &dyn Fn(&Address) -> u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        let senders: Vec<Address> = inner.senders.keys().cloned().collect();
        let mut hashes_to_remove = Vec::new();
        let mut empty_senders = Vec::new();

        for sender in &senders {
            let current_nonce = nonce_fn(sender);
            if let Some(queue) = inner.senders.get_mut(sender) {
                // Remove stale pending txs.
                let stale: Vec<u64> = queue
                    .pending
                    .range(..current_nonce)
                    .map(|(&n, _)| n)
                    .collect();
                for n in stale {
                    if let Some(entry) = queue.pending.remove(&n) {
                        hashes_to_remove.push(entry.verified.tx_hash);
                    }
                }

                // Remove stale queued txs.
                let stale: Vec<u64> = queue
                    .queued
                    .range(..current_nonce)
                    .map(|(&n, _)| n)
                    .collect();
                for n in stale {
                    if let Some(entry) = queue.queued.remove(&n) {
                        hashes_to_remove.push(entry.verified.tx_hash);
                    }
                }

                // Promote queued → pending.
                Self::promote_queued_static(queue, current_nonce);

                if queue.pending.is_empty() && queue.queued.is_empty() {
                    empty_senders.push(*sender);
                }
            }
        }

        for hash in hashes_to_remove {
            inner.by_hash.remove(&hash);
        }
        for sender in empty_senders {
            inner.senders.remove(&sender);
        }
    }

    /// Number of pending transactions across all senders.
    pub fn pending_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .senders
            .values()
            .map(|q| q.pending.len())
            .sum()
    }

    /// Number of queued transactions across all senders.
    pub fn queued_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .senders
            .values()
            .map(|q| q.queued.len())
            .sum()
    }

    /// Check if a tx hash is in the pool.
    pub fn contains(&self, hash: &B256) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .by_hash
            .contains_key(hash)
    }

    fn promote_queued_static(queue: &mut SenderQueue, current_nonce: u64) {
        let mut next = queue
            .pending
            .keys()
            .last()
            .map(|n| n + 1)
            .unwrap_or(current_nonce);

        while let Some(entry) = queue.queued.remove(&next) {
            queue.pending.insert(next, entry);
            next += 1;
        }
    }
}

/// Update the pool's base fee (e.g., after EIP-1559 adjustment).
impl EvmTxPool {
    pub fn update_base_fee(&self, new_base_fee: u64) {
        // Re-calculate effective tips is deferred — for now just log.
        debug!(new_base_fee, "txpool base fee updated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TxEnvelope;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_eips::Encodable2718;
    use alloy_primitives::{Bytes, Signature, TxKind, U256};

    fn make_eip1559_tx(key: &k256::ecdsa::SigningKey, nonce: u64, tip: u128) -> Vec<u8> {
        let mut tx = TxEip1559 {
            chain_id: 1337,
            nonce,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: tip,
            gas_limit: 21_000,
            to: TxKind::Call(Address::repeat_byte(0xBB)),
            value: U256::from(1_000_000u64),
            input: Bytes::new(),
            access_list: Default::default(),
        };

        let sig_hash = tx.signature_hash();
        let (sig, recid) = key
            .sign_prehash_recoverable(sig_hash.as_slice())
            .expect("sign");
        let signature = Signature::from_signature_and_parity(sig, recid.is_y_odd());
        let signed = tx.into_signed(signature);
        let envelope = TxEnvelope::Eip1559(signed);
        let mut buf = Vec::new();
        envelope.encode_2718(&mut buf);
        buf
    }

    #[test]
    fn test_submit_and_count() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 0));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let raw = make_eip1559_tx(&key, 0, 1_000_000_000);

        let hash = pool.submit_tx(&raw).expect("submit should succeed");
        assert!(pool.contains(&hash));
        assert_eq!(pool.pending_count(), 1);
        assert_eq!(pool.queued_count(), 0);
    }

    #[test]
    fn test_nonce_gap_goes_to_queued() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 0));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());

        // Submit nonce 2 (gap: 0 and 1 missing).
        let raw = make_eip1559_tx(&key, 2, 1_000_000_000);
        pool.submit_tx(&raw).expect("submit");
        assert_eq!(pool.pending_count(), 0);
        assert_eq!(pool.queued_count(), 1);

        // Fill nonce 0 → goes to pending.
        let raw0 = make_eip1559_tx(&key, 0, 1_000_000_000);
        pool.submit_tx(&raw0).expect("submit");
        assert_eq!(pool.pending_count(), 1);
        assert_eq!(pool.queued_count(), 1);

        // Fill nonce 1 → promotes nonce 2 from queued to pending.
        let raw1 = make_eip1559_tx(&key, 1, 1_000_000_000);
        pool.submit_tx(&raw1).expect("submit");
        assert_eq!(pool.pending_count(), 3);
        assert_eq!(pool.queued_count(), 0);
    }

    #[test]
    fn test_replacement_by_fee() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 0));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());

        let raw1 = make_eip1559_tx(&key, 0, 1_000_000_000);
        pool.submit_tx(&raw1).expect("submit 1");

        // Same nonce, higher tip → should replace.
        let raw2 = make_eip1559_tx(&key, 0, 2_000_000_000);
        pool.submit_tx(&raw2).expect("submit 2 (replacement)");
        assert_eq!(pool.pending_count(), 1);

        // Same nonce, lower tip → should fail.
        let raw3 = make_eip1559_tx(&key, 0, 500_000_000);
        let err = pool
            .submit_tx(&raw3)
            .expect_err("should reject underpriced");
        assert!(err.contains("replacement underpriced"));
    }

    #[test]
    fn test_nonce_too_low_rejected() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 5));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let raw = make_eip1559_tx(&key, 3, 1_000_000_000);
        let err = pool.submit_tx(&raw).expect_err("should reject low nonce");
        assert!(err.contains("nonce too low"));
    }

    #[test]
    fn test_collect_payload() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 0));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let raw = make_eip1559_tx(&key, 0, 1_000_000_000);
        pool.submit_tx(&raw).expect("submit");

        let payload = pool.collect_payload(1_000_000, 30_000_000);
        assert!(!payload.is_empty());
        assert_eq!(pool.pending_count(), 0); // collected txs are removed
    }

    #[test]
    fn test_on_commit_removes_stale() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 0));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let raw0 = make_eip1559_tx(&key, 0, 1_000_000_000);
        let raw1 = make_eip1559_tx(&key, 1, 1_000_000_000);
        pool.submit_tx(&raw0).expect("submit 0");
        pool.submit_tx(&raw1).expect("submit 1");
        assert_eq!(pool.pending_count(), 2);

        // Simulate commit: nonce is now 1 (tx 0 committed).
        pool.on_commit(&|_| 1);
        assert_eq!(pool.pending_count(), 1); // only nonce 1 remains
    }

    #[test]
    fn test_duplicate_rejected() {
        let pool = EvmTxPool::new(EvmTxPoolConfig::default());
        pool.set_nonce_fn(Box::new(|_| 0));

        let key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let raw = make_eip1559_tx(&key, 0, 1_000_000_000);
        pool.submit_tx(&raw).expect("submit 1");
        let err = pool.submit_tx(&raw).expect_err("should reject duplicate");
        assert!(err.contains("already known"));
    }
}
