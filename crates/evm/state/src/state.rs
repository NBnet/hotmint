use alloy_primitives::{Address, B256, U256};
use hotmint_evm_types::EvmChainConfig;
use hotmint_evm_types::genesis::EvmGenesis;
use revm::database::CacheDB;
use revm::database_interface::EmptyDB;
use revm::state::AccountInfo;
use std::collections::BTreeMap;
use vsdb::MptCalc;

/// Persistent EVM world state backed by vsdb.
///
/// Phase 0: Uses in-memory CacheDB + MptCalc (same as examples/evm-chain).
/// Phase 2: Will migrate to full vsdb MapxOrd-backed persistent storage.
pub struct EvmState {
    pub db: CacheDB<EmptyDB>,
    pub state_trie: MptCalc,
    pub config: EvmChainConfig,
    /// Block hashes for BLOCKHASH opcode (recent 256).
    pub block_hashes: BTreeMap<u64, B256>,
}

impl EvmState {
    /// Initialize from genesis.
    pub fn from_genesis(genesis: &EvmGenesis) -> Self {
        let mut db = CacheDB::new(EmptyDB::default());
        let mut state_trie = MptCalc::new();

        for (addr, alloc) in &genesis.alloc {
            let mut info = AccountInfo {
                balance: alloc.balance,
                nonce: alloc.nonce,
                ..Default::default()
            };
            if !alloc.code.is_empty() {
                info.code = Some(revm::bytecode::Bytecode::new_raw(
                    revm::primitives::Bytes::copy_from_slice(&alloc.code),
                ));
            }
            db.insert_account_info(*addr, info.clone());

            let encoded = encode_account_leaf(&info);
            let _ = state_trie.insert(addr.as_slice(), &encoded);
        }

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

    /// Get account balance.
    pub fn get_balance(&self, addr: &Address) -> U256 {
        self.db
            .cache
            .accounts
            .get(addr)
            .map(|a| a.info.balance)
            .unwrap_or_default()
    }

    /// Get account nonce.
    pub fn get_nonce(&self, addr: &Address) -> u64 {
        self.db
            .cache
            .accounts
            .get(addr)
            .map(|a| a.info.nonce)
            .unwrap_or(0)
    }

    /// Record a block hash for BLOCKHASH opcode.
    pub fn record_block_hash(&mut self, number: u64, hash: B256) {
        self.block_hashes.insert(number, hash);
        // Keep only recent 256
        if self.block_hashes.len() > 256 {
            let oldest = *self.block_hashes.keys().next().unwrap();
            self.block_hashes.remove(&oldest);
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
}
