use std::sync::Mutex;

use ruc::*;
use tracing::{info, warn};

use hotmint_consensus::application::{Application, TxValidationResult};
use hotmint_evm_state::EvmState;
use hotmint_evm_types::EvmChainConfig;
use hotmint_evm_types::genesis::EvmGenesis;
use hotmint_evm_types::tx;
use hotmint_types::Block;
use hotmint_types::block::BlockHash;
use hotmint_types::context::{BlockContext, TxContext};
use hotmint_types::validator_update::EndBlockResponse;
use alloy_consensus::Transaction;

/// EVM block executor implementing the Hotmint `Application` trait.
///
/// Decodes Ethereum signed transactions, validates signatures + chain state,
/// and executes them via revm.
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
    fn validate_tx(&self, raw: &[u8], _ctx: Option<&TxContext>) -> TxValidationResult {
        // Decode and recover sender.
        let verified = match tx::decode_and_recover(raw) {
            Ok(v) => v,
            Err(e) => {
                warn!("tx decode/recover failed: {e}");
                return TxValidationResult::reject();
            }
        };

        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Validate against chain state.
        if let Err(e) = tx::validate_tx(
            &verified,
            state.config.chain_id,
            state.get_nonce(&verified.sender),
            state.get_balance(&verified.sender),
            state.config.block_gas_limit,
            state.config.base_fee_per_gas,
        ) {
            warn!(sender = %verified.sender, "tx validation failed: {e}");
            return TxValidationResult::reject();
        }

        let priority = tx::effective_gas_tip(
            &verified.envelope,
            state.config.base_fee_per_gas,
        );
        let gas_wanted = verified.envelope.gas_limit();

        TxValidationResult::accept_with_gas(priority, gas_wanted)
    }

    fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Record parent block hash for BLOCKHASH opcode.
        if ctx.height.as_u64() > 0 {
            let parent_hash = alloy_primitives::B256::new(
                ctx.validator_set
                    .validators()
                    .first()
                    .map(|_| [0u8; 32]) // placeholder — real parent hash from store
                    .unwrap_or_default(),
            );
            state.record_block_hash(ctx.height.as_u64().saturating_sub(1), parent_hash);
        }

        // Decode all transactions.
        let mut verified_txs = Vec::with_capacity(txs.len());
        for raw in txs {
            match tx::decode_and_recover(raw) {
                Ok(v) => verified_txs.push(v),
                Err(e) => {
                    warn!("skipping invalid tx in block: {e}");
                    continue;
                }
            }
        }

        info!(
            height = ctx.height.as_u64(),
            valid_txs = verified_txs.len(),
            total_txs = txs.len(),
            "executing EVM block"
        );

        // Phase 1: Validate and apply nonce/balance changes (no full EVM yet).
        // Phase 3 will add real revm execution.
        for vtx in &verified_txs {
            let nonce = state.get_nonce(&vtx.sender);
            let tx_nonce = vtx.envelope.nonce();
            if tx_nonce != nonce {
                warn!(
                    sender = %vtx.sender,
                    expected = nonce,
                    got = tx_nonce,
                    "nonce mismatch in block execution"
                );
                continue;
            }

            // Increment nonce for accepted transactions.
            state.set_nonce(&vtx.sender, nonce.checked_add(1).unwrap_or(nonce));

            // Deduct intrinsic gas cost from balance (simplified — real deduction in Phase 3).
            let gas_cost = alloy_primitives::U256::from(vtx.envelope.gas_limit())
                * alloy_primitives::U256::from(vtx.envelope.max_fee_per_gas());
            let value = vtx.envelope.value();
            let total_deduct = gas_cost.saturating_add(value);

            let balance = state.get_balance(&vtx.sender);
            if balance >= total_deduct {
                state.set_balance(&vtx.sender, balance.saturating_sub(total_deduct));

                // Credit value to recipient (simple transfer).
                if let Some(to) = vtx.envelope.to() {
                    let to_balance = state.get_balance(&to);
                    state.set_balance(&to, to_balance.saturating_add(value));
                }
            }
        }

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
