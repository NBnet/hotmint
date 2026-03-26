use alloy_primitives::{Address, B256, U256};
use hotmint_evm_types::EvmChainConfig;
use hotmint_evm_types::genesis::EvmGenesis;
use revm::bytecode::Bytecode;
use revm::database::CacheDB;
use revm::database_interface::{DBErrorMarker, Database};
use revm::primitives::Bytes;
use revm::state::AccountInfo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use vsdb::{MapxOrdRawKey, MptCalc};

// ---------------------------------------------------------------------------
// Serializable wrappers for vsdb storage (postcard Serialize/Deserialize)
// ---------------------------------------------------------------------------

/// Compact serializable account info for vsdb persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    balance_be: [u8; 32],
    nonce: u64,
    code_hash: [u8; 32],
}

impl StoredAccount {
    fn from_info(info: &AccountInfo) -> Self {
        Self {
            balance_be: info.balance.to_be_bytes(),
            nonce: info.nonce,
            code_hash: info.code_hash.0,
        }
    }

    fn to_info(&self) -> AccountInfo {
        AccountInfo {
            balance: U256::from_be_bytes(self.balance_be),
            nonce: self.nonce,
            code_hash: B256::from(self.code_hash),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// VsdbStateDb — persistent backing store implementing revm::Database
// ---------------------------------------------------------------------------

/// Database error for vsdb operations.
#[derive(Debug)]
pub struct VsdbError(pub String);

impl std::fmt::Display for VsdbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vsdb: {}", self.0)
    }
}
impl std::error::Error for VsdbError {}
impl DBErrorMarker for VsdbError {}

/// Persistent EVM world state backed by vsdb `MapxOrdRawKey`.
///
/// Keys are raw bytes (Address=20, B256=32, composite=52) for deterministic
/// ordered storage. Values implement `Serialize + Deserialize` via postcard.
pub struct VsdbStateDb {
    /// Address(20B) → StoredAccount
    accounts: MapxOrdRawKey<StoredAccount>,
    /// code_hash(32B) → contract bytecode bytes
    contracts: MapxOrdRawKey<Vec<u8>>,
    /// address(20B) || slot(32B) = 52B → storage value (32B big-endian)
    storage: MapxOrdRawKey<[u8; 32]>,
    /// block_number(8B big-endian) → block hash (32B)
    block_hashes: MapxOrdRawKey<[u8; 32]>,
}

impl VsdbStateDb {
    /// Create a new empty vsdb state database.
    pub fn new() -> Self {
        Self {
            accounts: MapxOrdRawKey::new(),
            contracts: MapxOrdRawKey::new(),
            storage: MapxOrdRawKey::new(),
            block_hashes: MapxOrdRawKey::new(),
        }
    }

    /// Insert an account (for genesis initialization).
    pub fn insert_account(&mut self, addr: &Address, info: &AccountInfo) {
        self.accounts.insert(addr.as_slice(), &StoredAccount::from_info(info));
        if let Some(code) = &info.code {
            let bytes = code.bytes_slice().to_vec();
            if !bytes.is_empty() {
                self.contracts.insert(info.code_hash.as_slice(), &bytes);
            }
        }
    }

    /// Insert a storage slot.
    pub fn insert_storage(&mut self, addr: &Address, slot: &U256, value: &U256) {
        let key = storage_key(addr, slot);
        self.storage.insert(key.as_slice(), &value.to_be_bytes());
    }

    /// Insert a block hash.
    pub fn insert_block_hash(&mut self, number: u64, hash: &B256) {
        self.block_hashes.insert(&number.to_be_bytes(), &hash.0);
    }

    /// Get account info.
    pub fn get_account(&self, addr: &Address) -> Option<AccountInfo> {
        self.accounts.get(addr.as_slice()).map(|s: StoredAccount| s.to_info())
    }

    /// Get account balance.
    pub fn get_balance(&self, addr: &Address) -> U256 {
        self.accounts
            .get(addr.as_slice())
            .map(|s: StoredAccount| U256::from_be_bytes(s.balance_be))
            .unwrap_or_default()
    }

    /// Get account nonce.
    pub fn get_nonce(&self, addr: &Address) -> u64 {
        self.accounts
            .get(addr.as_slice())
            .map(|s: StoredAccount| s.nonce)
            .unwrap_or(0)
    }

    /// Set account balance.
    pub fn set_balance(&mut self, addr: &Address, balance: U256) {
        let mut stored = self
            .accounts
            .get(addr.as_slice())
            .unwrap_or(StoredAccount {
                balance_be: [0u8; 32],
                nonce: 0,
                code_hash: revm::primitives::KECCAK_EMPTY.0,
            });
        stored.balance_be = balance.to_be_bytes();
        self.accounts.insert(addr.as_slice(), &stored);
    }

    /// Set account nonce.
    pub fn set_nonce(&mut self, addr: &Address, nonce: u64) {
        let mut stored = self
            .accounts
            .get(addr.as_slice())
            .unwrap_or(StoredAccount {
                balance_be: [0u8; 32],
                nonce: 0,
                code_hash: revm::primitives::KECCAK_EMPTY.0,
            });
        stored.nonce = nonce;
        self.accounts.insert(addr.as_slice(), &stored);
    }
}

impl Default for VsdbStateDb {
    fn default() -> Self {
        Self::new()
    }
}

impl Database for VsdbStateDb {
    type Error = VsdbError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.accounts.get(address.as_slice()).map(|s: StoredAccount| {
            let mut info = s.to_info();
            // Load code if this account has non-empty code.
            if info.code_hash != revm::primitives::KECCAK_EMPTY {
                if let Some(bytes) = self.contracts.get(info.code_hash.as_slice()) {
                    info.code = Some(Bytecode::new_raw(Bytes::from(bytes)));
                }
            }
            info
        }))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self.contracts.get(code_hash.as_slice()) {
            Some(bytes) => Ok(Bytecode::new_raw(Bytes::from(bytes))),
            None => Ok(Bytecode::default()),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let key = storage_key(&address, &index);
        match self.storage.get(key.as_slice()) {
            Some(be_bytes) => Ok(U256::from_be_bytes(be_bytes)),
            None => Ok(U256::ZERO),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self.block_hashes.get(&number.to_be_bytes()) {
            Some(h) => Ok(B256::from(h)),
            None => Ok(B256::ZERO),
        }
    }
}

/// Build composite storage key: address(20) || slot(32) = 52 bytes.
fn storage_key(addr: &Address, slot: &U256) -> [u8; 52] {
    let mut key = [0u8; 52];
    key[..20].copy_from_slice(addr.as_slice());
    key[20..].copy_from_slice(&slot.to_be_bytes::<32>());
    key
}

// ---------------------------------------------------------------------------
// EvmState — high-level state with CacheDB + vsdb + state trie
// ---------------------------------------------------------------------------

/// Persistent EVM world state.
///
/// Uses `CacheDB<VsdbStateDb>` for execution (CacheDB provides in-memory
/// caching during block execution, VsdbStateDb provides persistence).
/// State root is computed from an MPT trie updated on every account change.
pub struct EvmState {
    pub db: CacheDB<VsdbStateDb>,
    pub state_trie: MptCalc,
    pub config: EvmChainConfig,
    /// Recent block hashes (for BLOCKHASH opcode) — kept in both CacheDB cache
    /// and vsdb for persistence across restarts.
    pub block_hashes: BTreeMap<u64, B256>,
}

impl EvmState {
    /// Initialize from genesis.
    pub fn from_genesis(genesis: &EvmGenesis) -> Self {
        let mut vsdb_db = VsdbStateDb::new();
        let mut state_trie = MptCalc::new();

        for (addr, alloc) in &genesis.alloc {
            let mut info = AccountInfo {
                balance: alloc.balance,
                nonce: alloc.nonce,
                ..Default::default()
            };
            if !alloc.code.is_empty() {
                info.code = Some(Bytecode::new_raw(Bytes::copy_from_slice(&alloc.code)));
            }

            // Persist to vsdb.
            vsdb_db.insert_account(addr, &info);

            // Initialize genesis storage.
            for (slot, value) in &alloc.storage {
                vsdb_db.insert_storage(addr, slot, value);
            }

            // Update trie.
            let encoded = encode_account_leaf(&info);
            let _ = state_trie.insert(addr.as_slice(), &encoded);
        }

        let db = CacheDB::new(vsdb_db);

        let config = EvmChainConfig {
            chain_id: genesis.chain_id,
            block_gas_limit: genesis.gas_limit,
            base_fee_per_gas: genesis.base_fee_per_gas,
            ..Default::default()
        };

        Self {
            db,
            state_trie,
            config,
            block_hashes: BTreeMap::new(),
        }
    }

    /// Compute the current state root hash.
    pub fn state_root(&mut self) -> [u8; 32] {
        let root = self.state_trie.root_hash().unwrap_or_default();
        let mut arr = [0u8; 32];
        let len = root.len().min(32);
        arr[..len].copy_from_slice(&root[..len]);
        arr
    }

    /// Get account balance (reads from cache first, then vsdb).
    pub fn get_balance(&self, addr: &Address) -> U256 {
        // Try CacheDB cache first.
        if let Some(acc) = self.db.cache.accounts.get(addr) {
            return acc.info.balance;
        }
        // Fall back to vsdb.
        self.db.db.get_balance(addr)
    }

    /// Get account nonce (reads from cache first, then vsdb).
    pub fn get_nonce(&self, addr: &Address) -> u64 {
        if let Some(acc) = self.db.cache.accounts.get(addr) {
            return acc.info.nonce;
        }
        self.db.db.get_nonce(addr)
    }

    /// Set account nonce and update both cache and trie.
    pub fn set_nonce(&mut self, addr: &Address, nonce: u64) {
        let entry = self.db.cache.accounts.entry(*addr).or_default();
        entry.info.nonce = nonce;
        // Also persist to vsdb.
        self.db.db.set_nonce(addr, nonce);
        let encoded = encode_account_leaf(&entry.info);
        let _ = self.state_trie.insert(addr.as_slice(), &encoded);
    }

    /// Set account balance and update both cache and trie.
    pub fn set_balance(&mut self, addr: &Address, balance: U256) {
        let entry = self.db.cache.accounts.entry(*addr).or_default();
        entry.info.balance = balance;
        self.db.db.set_balance(addr, balance);
        let encoded = encode_account_leaf(&entry.info);
        let _ = self.state_trie.insert(addr.as_slice(), &encoded);
    }

    /// Record a block hash for BLOCKHASH opcode.
    pub fn record_block_hash(&mut self, number: u64, hash: B256) {
        self.block_hashes.insert(number, hash);
        self.db.db.insert_block_hash(number, &hash);
        // Keep only recent 256 in memory.
        if self.block_hashes.len() > 256 {
            let oldest = *self.block_hashes.keys().next().unwrap();
            self.block_hashes.remove(&oldest);
        }
    }

    /// Flush CacheDB changes to vsdb after block execution.
    /// Called after revm `transact_commit` to persist state changes.
    pub fn flush_cache_to_vsdb(&mut self) {
        for (addr, cached_acc) in &self.db.cache.accounts {
            let info = &cached_acc.info;
            self.db.db.insert_account(addr, info);

            // Persist storage changes.
            for (slot, value) in &cached_acc.storage {
                self.db.db.insert_storage(addr, slot, value);
            }

            // Update trie.
            let encoded = encode_account_leaf(info);
            let _ = self.state_trie.insert(addr.as_slice(), &encoded);
        }
    }
}

/// Encode account state for trie: `nonce(8) || balance(32) || code_hash(32)`.
fn encode_account_leaf(info: &AccountInfo) -> Vec<u8> {
    let mut buf = Vec::with_capacity(72);
    buf.extend_from_slice(&info.nonce.to_be_bytes());
    buf.extend_from_slice(&info.balance.to_be_bytes::<32>());
    buf.extend_from_slice(info.code_hash.as_slice());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_genesis_empty() {
        let genesis = EvmGenesis {
            chain_id: 1337,
            alloc: BTreeMap::new(),
            gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000,
            coinbase: Address::default(),
            timestamp: 0,
        };
        let state = EvmState::from_genesis(&genesis);
        assert_eq!(state.config.chain_id, 1337);
    }

    #[test]
    fn test_from_genesis_with_alloc() {
        let mut alloc = BTreeMap::new();
        let addr = Address::repeat_byte(0xAA);
        alloc.insert(
            addr,
            hotmint_evm_types::genesis::GenesisAlloc {
                balance: U256::from(1_000_000_000_000_000_000u128),
                nonce: 0,
                code: vec![],
                storage: BTreeMap::new(),
            },
        );
        let genesis = EvmGenesis {
            chain_id: 1337,
            alloc,
            gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000,
            coinbase: Address::default(),
            timestamp: 0,
        };
        let state = EvmState::from_genesis(&genesis);
        assert_eq!(
            state.get_balance(&addr),
            U256::from(1_000_000_000_000_000_000u128)
        );
    }

    #[test]
    fn test_vsdb_database_basic() {
        let mut db = VsdbStateDb::new();
        let addr = Address::repeat_byte(0x11);
        let info = AccountInfo {
            balance: U256::from(42u64),
            nonce: 7,
            ..Default::default()
        };
        db.insert_account(&addr, &info);

        let loaded = db.basic(addr).unwrap().unwrap();
        assert_eq!(loaded.balance, U256::from(42u64));
        assert_eq!(loaded.nonce, 7);
    }

    #[test]
    fn test_vsdb_database_storage() {
        let mut db = VsdbStateDb::new();
        let addr = Address::repeat_byte(0x22);
        let slot = U256::from(5u64);
        let value = U256::from(999u64);
        db.insert_storage(&addr, &slot, &value);

        let loaded = db.storage(addr, slot).unwrap();
        assert_eq!(loaded, value);

        let empty = db.storage(addr, U256::from(99u64)).unwrap();
        assert_eq!(empty, U256::ZERO);
    }

    #[test]
    fn test_vsdb_database_block_hash() {
        let mut db = VsdbStateDb::new();
        let hash = B256::repeat_byte(0xCC);
        db.insert_block_hash(42, &hash);

        let loaded = db.block_hash(42).unwrap();
        assert_eq!(loaded, hash);

        let missing = db.block_hash(99).unwrap();
        assert_eq!(missing, B256::ZERO);
    }

    #[test]
    fn test_set_balance_and_nonce() {
        let genesis = EvmGenesis {
            chain_id: 1337,
            alloc: BTreeMap::new(),
            gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000,
            coinbase: Address::default(),
            timestamp: 0,
        };
        let mut state = EvmState::from_genesis(&genesis);
        let addr = Address::repeat_byte(0x33);

        state.set_balance(&addr, U256::from(100u64));
        assert_eq!(state.get_balance(&addr), U256::from(100u64));

        state.set_nonce(&addr, 5);
        assert_eq!(state.get_nonce(&addr), 5);

        // Verify vsdb persistence.
        assert_eq!(state.db.db.get_balance(&addr), U256::from(100u64));
        assert_eq!(state.db.db.get_nonce(&addr), 5);
    }

    #[test]
    fn test_state_root_determinism() {
        let mut alloc = BTreeMap::new();
        let addr = Address::repeat_byte(0xAA);
        alloc.insert(
            addr,
            hotmint_evm_types::genesis::GenesisAlloc {
                balance: U256::from(1000u64),
                nonce: 0,
                code: vec![],
                storage: BTreeMap::new(),
            },
        );
        let genesis = EvmGenesis {
            chain_id: 1337,
            alloc: alloc.clone(),
            gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000,
            coinbase: Address::default(),
            timestamp: 0,
        };

        let mut state1 = EvmState::from_genesis(&genesis);
        let mut state2 = EvmState::from_genesis(&genesis);

        let root1 = state1.state_root();
        let root2 = state2.state_root();
        assert_eq!(root1, root2, "same genesis must produce same state root");
        assert_ne!(root1, [0u8; 32], "state root should not be all zeros");
    }
}
