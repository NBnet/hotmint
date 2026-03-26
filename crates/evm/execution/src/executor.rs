use std::sync::Mutex;

use ruc::*;
use tracing::{info, warn};

use hotmint_consensus::application::{Application, TxValidationResult};
use hotmint_evm_state::EvmState;
use hotmint_evm_types::EvmChainConfig;
use hotmint_evm_types::genesis::EvmGenesis;
use hotmint_evm_types::receipt::{EvmLog, EvmReceipt};
use hotmint_evm_types::tx;
use hotmint_types::Block;
use hotmint_types::block::BlockHash;
use hotmint_types::context::{BlockContext, TxContext};
use hotmint_types::validator_update::EndBlockResponse;

use alloy_consensus::Transaction;
use alloy_primitives::{Address, B256, Bytes, U256};
use revm::context::TxEnv;
use revm::handler::ExecuteCommitEvm;
use revm::primitives::{hardfork::SpecId, TxKind};
use revm::{Context, MainBuilder, MainContext};

/// EVM block executor implementing the Hotmint `Application` trait.
pub struct EvmExecutor {
    state: Mutex<EvmState>,
    /// Accumulated receipts per block (for RPC queries).
    receipts: Mutex<Vec<Vec<EvmReceipt>>>,
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
            receipts: Mutex::new(Vec::new()),
        }
    }

    /// Get the current chain config.
    pub fn config(&self) -> EvmChainConfig {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .config
            .clone()
    }

    /// Get receipts for a block by index (0-based).
    pub fn get_receipts(&self, block_index: usize) -> Option<Vec<EvmReceipt>> {
        self.receipts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(block_index)
            .cloned()
    }
}

impl Application for EvmExecutor {
    fn validate_tx(&self, raw: &[u8], _ctx: Option<&TxContext>) -> TxValidationResult {
        let verified = match tx::decode_and_recover(raw) {
            Ok(v) => v,
            Err(e) => {
                warn!("tx decode/recover failed: {e}");
                return TxValidationResult::reject();
            }
        };

        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());

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

        let priority = tx::effective_gas_tip(&verified.envelope, state.config.base_fee_per_gas);
        let gas_wanted = verified.envelope.gas_limit();

        TxValidationResult::accept_with_gas(priority, gas_wanted)
    }

    fn execute_block(&self, txs: &[&[u8]], ctx: &BlockContext) -> Result<EndBlockResponse> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let config = state.config.clone();

        // Record parent block hash for BLOCKHASH opcode.
        if ctx.height.as_u64() > 0 {
            let parent_hash = B256::ZERO; // placeholder — real parent hash from block store
            state.record_block_hash(ctx.height.as_u64().saturating_sub(1), parent_hash);
        }

        // Decode all transactions.
        let mut verified_txs = Vec::with_capacity(txs.len());
        for raw in txs {
            match tx::decode_and_recover(raw) {
                Ok(v) => verified_txs.push(v),
                Err(e) => {
                    warn!("skipping invalid tx in block: {e}");
                }
            }
        }

        let mut cumulative_gas_used: u64 = 0;
        let mut block_receipts = Vec::with_capacity(verified_txs.len());

        // Coinbase/beneficiary — derive from proposer validator ID.
        let coinbase = {
            let mut addr_bytes = [0u8; 20];
            let id_bytes = ctx.proposer.0.to_be_bytes();
            addr_bytes[12..20].copy_from_slice(&id_bytes);
            Address::from(addr_bytes)
        };

        // Execute each transaction via revm.
        for (tx_idx, vtx) in verified_txs.iter().enumerate() {
            let expected_nonce = state.get_nonce(&vtx.sender);
            let tx_nonce = vtx.envelope.nonce();
            if tx_nonce != expected_nonce {
                warn!(
                    sender = %vtx.sender,
                    expected = expected_nonce,
                    got = tx_nonce,
                    "nonce mismatch, skipping tx"
                );
                continue;
            }

            let tx_gas = vtx.envelope.gas_limit();
            if cumulative_gas_used.saturating_add(tx_gas) > config.block_gas_limit {
                warn!(
                    tx_idx,
                    cumulative_gas_used,
                    tx_gas,
                    limit = config.block_gas_limit,
                    "block gas limit exceeded"
                );
                break;
            }

            // Build TxEnv from decoded transaction.
            let tx_env = TxEnv {
                tx_type: vtx.envelope.tx_type() as u8,
                caller: vtx.sender,
                gas_limit: tx_gas,
                gas_price: vtx.envelope.max_fee_per_gas(),
                gas_priority_fee: vtx.envelope.max_priority_fee_per_gas(),
                kind: match vtx.envelope.to() {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                value: vtx.envelope.value(),
                data: vtx.envelope.input().clone(),
                nonce: tx_nonce,
                chain_id: vtx.envelope.chain_id(),
                access_list: Default::default(),
                ..Default::default()
            };

            // Build revm context and execute.
            let evm_ctx = Context::mainnet()
                .with_db(&mut state.db)
                .modify_cfg_chained(|cfg| {
                    cfg.chain_id = config.chain_id;
                    cfg.set_spec_and_mainnet_gas_params(SpecId::CANCUN);
                })
                .modify_block_chained(|block| {
                    block.number = U256::from(ctx.height.as_u64());
                    block.beneficiary = coinbase;
                    block.timestamp = U256::ZERO; // TODO: use real block timestamp
                    block.gas_limit = config.block_gas_limit;
                    block.basefee = config.base_fee_per_gas;
                });

            let mut evm = evm_ctx.build_mainnet();

            match evm.transact_commit(tx_env) {
                Ok(result) => {
                    let gas_used = result.gas_used();
                    cumulative_gas_used = cumulative_gas_used.saturating_add(gas_used);

                    let (success, logs) = match &result {
                        revm::context_interface::result::ExecutionResult::Success {
                            logs, ..
                        } => (true, logs.clone()),
                        revm::context_interface::result::ExecutionResult::Revert {
                            logs, ..
                        } => (false, logs.clone()),
                        revm::context_interface::result::ExecutionResult::Halt { .. } => {
                            (false, vec![])
                        }
                    };

                    // Compute effective gas price.
                    let effective_gas_price = {
                        let base = config.base_fee_per_gas as u128;
                        let max_fee = vtx.envelope.max_fee_per_gas();
                        let max_priority = vtx.envelope.max_priority_fee_per_gas().unwrap_or(0);
                        let tip = max_fee.saturating_sub(base).min(max_priority);
                        U256::from(base.saturating_add(tip))
                    };

                    let receipt = EvmReceipt {
                        tx_hash: vtx.tx_hash,
                        tx_index: tx_idx as u64,
                        block_hash: B256::ZERO, // filled after block finalization
                        block_number: ctx.height.as_u64(),
                        from: vtx.sender,
                        to: vtx.envelope.to(),
                        cumulative_gas_used,
                        gas_used,
                        effective_gas_price,
                        status: if success { 1 } else { 0 },
                        logs: logs
                            .iter()
                            .map(|log| EvmLog {
                                address: log.address,
                                topics: log.data.topics().to_vec(),
                                data: Bytes::copy_from_slice(&log.data.data),
                            })
                            .collect(),
                        logs_bloom: [0u8; 256],
                        contract_address: if vtx.envelope.to().is_none() {
                            Some(vtx.sender.create(tx_nonce))
                        } else {
                            None
                        },
                    };

                    block_receipts.push(receipt);
                }
                Err(e) => {
                    warn!(
                        tx_idx,
                        sender = %vtx.sender,
                        error = ?e,
                        "EVM tx execution failed"
                    );
                }
            }
        }

        info!(
            height = ctx.height.as_u64(),
            executed = block_receipts.len(),
            total_txs = txs.len(),
            cumulative_gas_used,
            "EVM block executed"
        );

        // Flush CacheDB changes to vsdb for persistence.
        state.flush_cache_to_vsdb();

        // Compute state root.
        let state_root = state.state_root();

        // Store receipts.
        self.receipts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(block_receipts);

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
                let addr = Address::from_slice(data);
                let bal = state.get_balance(&addr);
                bal.to_be_bytes::<32>().to_vec()
            }
            "eth_getTransactionCount" if data.len() == 20 => {
                let addr = Address::from_slice(data);
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
    use std::collections::BTreeMap;

    fn test_genesis() -> EvmGenesis {
        let mut alloc = BTreeMap::new();
        alloc.insert(
            Address::repeat_byte(0xAA),
            hotmint_evm_types::genesis::GenesisAlloc {
                balance: U256::from(1_000_000_000_000_000_000u128),
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
