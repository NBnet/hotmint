use std::sync::Mutex;

use ruc::*;
use tracing::info;

use hotmint_consensus::application::Application;
use hotmint_evm_state::EvmState;
use hotmint_evm_types::EvmChainConfig;
use hotmint_evm_types::genesis::EvmGenesis;
use hotmint_types::Block;
use hotmint_types::block::BlockHash;
use hotmint_types::context::BlockContext;
use hotmint_types::validator_update::EndBlockResponse;

/// EVM block executor implementing the Hotmint `Application` trait.
///
/// Phase 0: Minimal skeleton — creates state from genesis, executes blocks
/// using revm with simplified (postcard/CBOR) transactions.
/// Phase 1+: Will switch to real Ethereum RLP transactions.
pub struct EvmExecutor {
    state: Mutex<EvmState>,
}

impl EvmExecutor {
    /// Create a new executor from genesis configuration.
    pub fn from_genesis(genesis: &EvmGenesis) -> Self {
        let state = EvmState::from_genesis(genesis);
        info!(
            chain_id = state.config.chain_id,
            accounts = genesis.alloc.len(),
            gas_limit = state.config.block_gas_limit,
            "EVM executor initialized from genesis"
        );
        Self {
            state: Mutex::new(state),
        }
    }

    /// Get the current chain config.
    pub fn config(&self) -> EvmChainConfig {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).config.clone()
    }
}

impl Application for EvmExecutor {
    fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Record parent block hash for BLOCKHASH opcode.
        if ctx.height.as_u64() > 0 {
            let parent_hash = alloy_primitives::B256::new(
                ctx.validator_set
                    .validators()
                    .first()
                    .map(|_| [0u8; 32]) // placeholder — real parent hash comes from block
                    .unwrap_or_default(),
            );
            state.record_block_hash(ctx.height.as_u64().saturating_sub(1), parent_hash);
        }

        // Phase 0: Log block info. Full EVM execution comes in Phase 1-3.
        info!(
            height = ctx.height.as_u64(),
            tx_count = txs.len(),
            "executing EVM block"
        );

        // Compute state root for app_hash.
        let state_root = state.state_root();

        Ok(EndBlockResponse {
            app_hash: BlockHash(state_root),
            ..Default::default()
        })
    }

    fn on_commit(&self, _block: &Block, ctx: &BlockContext) -> Result<()> {
        info!(height = ctx.height.as_u64(), "EVM block committed");
        Ok(())
    }

    fn query(&self, path: &str, data: &[u8]) -> Result<hotmint_types::QueryResponse> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let result = match path {
            "eth_getBalance" if data.len() == 20 => {
                let addr = alloy_primitives::Address::from_slice(data);
                let bal = state.get_balance(&addr);
                bal.to_be_bytes::<32>().to_vec()
            }
            "eth_getTransactionCount" if data.len() == 20 => {
                let addr = alloy_primitives::Address::from_slice(data);
                let nonce = state.get_nonce(&addr);
                nonce.to_be_bytes().to_vec()
            }
            _ => vec![],
        };
        Ok(hotmint_types::QueryResponse {
            data: result,
            proof: None,
            height: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;
    use std::collections::BTreeMap;

    fn test_genesis() -> EvmGenesis {
        let mut alloc = BTreeMap::new();
        alloc.insert(
            Address::repeat_byte(0xAA),
            hotmint_evm_types::genesis::GenesisAlloc {
                balance: alloy_primitives::U256::from(1_000_000_000_000_000_000u128),
                nonce: 0,
                code: vec![],
                storage: BTreeMap::new(),
            },
        );
        EvmGenesis {
            chain_id: 1337,
            alloc,
            gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000,
            coinbase: Address::default(),
            timestamp: 0,
        }
    }

    #[test]
    fn test_executor_creation() {
        let executor = EvmExecutor::from_genesis(&test_genesis());
        assert_eq!(executor.config().chain_id, 1337);
    }
}
