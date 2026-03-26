//! Custom EVM precompiles bridging to native Hotmint modules.
//!
//! - 0x0800: Balances (native token balance query + transfer)
//! - 0x0801: Staking (delegate / unbond / claim / query)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, Bytes, U256};
use revm::context::Cfg;
use revm::context::LocalContextTr;
use revm::context_interface::{ContextTr, JournalTr};
use revm::handler::EthPrecompiles;
use revm::handler::PrecompileProvider;
use revm::interpreter::{CallInput, CallInputs, Gas, InstructionResult, InterpreterResult};
use revm::primitives::hardfork::SpecId;

/// Address of the Balances precompile.
pub const BALANCES_ADDR: Address = Address::new({
    let mut a = [0u8; 20];
    a[18] = 0x08;
    a[19] = 0x00;
    a
});

/// Address of the Staking precompile.
pub const STAKING_ADDR: Address = Address::new({
    let mut a = [0u8; 20];
    a[18] = 0x08;
    a[19] = 0x01;
    a
});

/// Gas cost for simple precompile queries.
const PRECOMPILE_BASE_GAS: u64 = 150;
/// Gas cost for state-modifying precompile operations.
const PRECOMPILE_WRITE_GAS: u64 = 500;

// ----- Staking state (shared across precompile calls) -----

/// Simple delegation record: (delegator, validator) → amount.
#[derive(Debug, Clone, Default)]
pub struct StakingState {
    delegations: HashMap<(Address, Address), U256>,
}

impl StakingState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn delegate(&mut self, delegator: Address, validator: Address, amount: U256) {
        let entry = self.delegations.entry((delegator, validator)).or_default();
        *entry = entry.saturating_add(amount);
    }

    pub fn unbond(
        &mut self,
        delegator: Address,
        validator: Address,
        amount: U256,
    ) -> std::result::Result<(), &'static str> {
        let entry = self.delegations.entry((delegator, validator)).or_default();
        if *entry < amount {
            return Err("insufficient delegation");
        }
        *entry = entry.saturating_sub(amount);
        Ok(())
    }

    pub fn get_stake(&self, delegator: &Address, validator: &Address) -> U256 {
        self.delegations
            .get(&(*delegator, *validator))
            .copied()
            .unwrap_or_default()
    }

    pub fn total_stake(&self, validator: &Address) -> U256 {
        self.delegations
            .iter()
            .filter(|((_, v), _)| v == validator)
            .fold(U256::ZERO, |acc, (_, amt)| acc.saturating_add(*amt))
    }
}

/// Shared staking state accessible from precompile calls.
pub type SharedStakingState = Arc<Mutex<StakingState>>;

// ----- Custom precompile provider -----

/// Precompile provider combining standard Ethereum precompiles with
/// Hotmint-specific Balances and Staking precompiles.
pub struct HotmintPrecompiles {
    eth: EthPrecompiles,
    staking: SharedStakingState,
}

impl HotmintPrecompiles {
    pub fn new(spec: SpecId, staking: SharedStakingState) -> Self {
        Self {
            eth: EthPrecompiles::new(spec),
            staking,
        }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for HotmintPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.eth, spec)
    }

    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> std::result::Result<Option<InterpreterResult>, String> {
        let target = inputs.bytecode_address;

        if target == BALANCES_ADDR {
            return Ok(Some(self.run_balances(context, inputs)));
        }
        if target == STAKING_ADDR {
            return Ok(Some(self.run_staking(context, inputs)));
        }

        // Delegate to standard Ethereum precompiles.
        self.eth.run(context, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let eth_addrs: Vec<Address> = self.eth.warm_addresses().collect();
        let custom = vec![BALANCES_ADDR, STAKING_ADDR];
        Box::new(eth_addrs.into_iter().chain(custom))
    }

    fn contains(&self, address: &Address) -> bool {
        *address == BALANCES_ADDR
            || *address == STAKING_ADDR
            || self.eth.contains(address)
    }
}

// ----- ABI function selectors (first 4 bytes of keccak256) -----

// Balances precompile selectors:
//   balanceOf(address) = 0x70a08231
//   transfer(address,uint256) = 0xa9059cbb
const SEL_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

// Staking precompile selectors:
//   delegate(address,uint256) = 0x026e402b
//   unbond(address,uint256) = 0xa5d059ca
//   getStake(address,address) = 0x82dda22d
//   totalStake(address) = 0xb273fc9a
const SEL_DELEGATE: [u8; 4] = [0x02, 0x6e, 0x40, 0x2b];
const SEL_UNBOND: [u8; 4] = [0xa5, 0xd0, 0x59, 0xca];
const SEL_GET_STAKE: [u8; 4] = [0x82, 0xdd, 0xa2, 0x2d];
const SEL_TOTAL_STAKE: [u8; 4] = [0xb2, 0x73, 0xfc, 0x9a];

impl HotmintPrecompiles {
    /// Execute Balances precompile (0x0800).
    fn run_balances<CTX: ContextTr>(
        &self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> InterpreterResult {
        let input = extract_input(context, inputs);
        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas: Gas::new(inputs.gas_limit),
            output: Bytes::new(),
        };

        if input.len() < 4 {
            result.result = InstructionResult::PrecompileError;
            return result;
        }

        let selector: [u8; 4] = input[..4].try_into().unwrap();

        match selector {
            SEL_BALANCE_OF => {
                // balanceOf(address) → uint256
                if input.len() < 36 {
                    result.result = InstructionResult::PrecompileError;
                    return result;
                }
                if !result.gas.record_cost(PRECOMPILE_BASE_GAS) {
                    result.result = InstructionResult::PrecompileOOG;
                    return result;
                }
                let addr = Address::from_slice(&input[16..36]);
                let balance = match context.journal_mut().load_account(addr) {
                    Ok(acc) => acc.data.info.balance,
                    Err(_) => U256::ZERO,
                };
                result.output = Bytes::copy_from_slice(&balance.to_be_bytes::<32>());
            }
            SEL_TRANSFER => {
                // transfer(address,uint256) → bool
                if input.len() < 68 {
                    result.result = InstructionResult::PrecompileError;
                    return result;
                }
                if !result.gas.record_cost(PRECOMPILE_WRITE_GAS) {
                    result.result = InstructionResult::PrecompileOOG;
                    return result;
                }
                let to = Address::from_slice(&input[16..36]);
                let amount = U256::from_be_slice(&input[36..68]);
                let caller = inputs.caller;

                // Perform transfer via journal.
                match context.journal_mut().transfer(caller, to, amount) {
                    Ok(None) => {
                        // Success.
                        result.output =
                            Bytes::copy_from_slice(&U256::from(1).to_be_bytes::<32>());
                    }
                    Ok(Some(_transfer_error)) => {
                        // Transfer error (e.g., insufficient balance).
                        result.result = InstructionResult::Revert;
                        result.output =
                            Bytes::copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
                    }
                    Err(_db_error) => {
                        result.result = InstructionResult::Revert;
                        result.output =
                            Bytes::copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
                    }
                }
            }
            _ => {
                result.result = InstructionResult::PrecompileError;
            }
        }

        result
    }

    /// Execute Staking precompile (0x0801).
    fn run_staking<CTX: ContextTr>(
        &self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> InterpreterResult {
        let input = extract_input(context, inputs);
        let mut result = InterpreterResult {
            result: InstructionResult::Return,
            gas: Gas::new(inputs.gas_limit),
            output: Bytes::new(),
        };

        if input.len() < 4 {
            result.result = InstructionResult::PrecompileError;
            return result;
        }

        let selector: [u8; 4] = input[..4].try_into().unwrap();

        match selector {
            SEL_DELEGATE => {
                // delegate(address validator, uint256 amount)
                if input.len() < 68 {
                    result.result = InstructionResult::PrecompileError;
                    return result;
                }
                if !result.gas.record_cost(PRECOMPILE_WRITE_GAS) {
                    result.result = InstructionResult::PrecompileOOG;
                    return result;
                }
                let validator = Address::from_slice(&input[16..36]);
                let amount = U256::from_be_slice(&input[36..68]);
                let delegator = inputs.caller;

                // Transfer tokens from delegator to staking address (lock).
                match context.journal_mut().transfer(delegator, STAKING_ADDR, amount) {
                    Ok(None) => {}
                    _ => {
                        result.result = InstructionResult::Revert;
                        result.output =
                            Bytes::copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
                        return result;
                    }
                }

                // Record delegation in staking state.
                self.staking
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .delegate(delegator, validator, amount);

                result.output = Bytes::copy_from_slice(&U256::from(1).to_be_bytes::<32>());
            }
            SEL_UNBOND => {
                // unbond(address validator, uint256 amount)
                if input.len() < 68 {
                    result.result = InstructionResult::PrecompileError;
                    return result;
                }
                if !result.gas.record_cost(PRECOMPILE_WRITE_GAS) {
                    result.result = InstructionResult::PrecompileOOG;
                    return result;
                }
                let validator = Address::from_slice(&input[16..36]);
                let amount = U256::from_be_slice(&input[36..68]);
                let delegator = inputs.caller;

                // Check and unbond from staking state first.
                {
                    let mut staking = self.staking.lock().unwrap_or_else(|e| e.into_inner());
                    if staking.unbond(delegator, validator, amount).is_err() {
                        result.result = InstructionResult::Revert;
                        result.output =
                            Bytes::copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
                        return result;
                    }
                }

                // Return tokens from staking address to delegator.
                match context.journal_mut().transfer(STAKING_ADDR, delegator, amount) {
                    Ok(None) => {}
                    _ => {
                        result.result = InstructionResult::Revert;
                        result.output =
                            Bytes::copy_from_slice(&U256::ZERO.to_be_bytes::<32>());
                        return result;
                    }
                }

                result.output = Bytes::copy_from_slice(&U256::from(1).to_be_bytes::<32>());
            }
            SEL_GET_STAKE => {
                // getStake(address delegator, address validator) → uint256
                if input.len() < 68 {
                    result.result = InstructionResult::PrecompileError;
                    return result;
                }
                if !result.gas.record_cost(PRECOMPILE_BASE_GAS) {
                    result.result = InstructionResult::PrecompileOOG;
                    return result;
                }
                let delegator = Address::from_slice(&input[16..36]);
                let validator = Address::from_slice(&input[48..68]);
                let stake = self
                    .staking
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_stake(&delegator, &validator);
                result.output = Bytes::copy_from_slice(&stake.to_be_bytes::<32>());
            }
            SEL_TOTAL_STAKE => {
                // totalStake(address validator) → uint256
                if input.len() < 36 {
                    result.result = InstructionResult::PrecompileError;
                    return result;
                }
                if !result.gas.record_cost(PRECOMPILE_BASE_GAS) {
                    result.result = InstructionResult::PrecompileOOG;
                    return result;
                }
                let validator = Address::from_slice(&input[16..36]);
                let total = self
                    .staking
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .total_stake(&validator);
                result.output = Bytes::copy_from_slice(&total.to_be_bytes::<32>());
            }
            _ => {
                result.result = InstructionResult::PrecompileError;
            }
        }

        result
    }
}

/// Extract input bytes from CallInputs.
fn extract_input<CTX: ContextTr>(context: &CTX, inputs: &CallInputs) -> Vec<u8> {
    match &inputs.input {
        CallInput::SharedBuffer(range) => context
            .local()
            .shared_memory_buffer_slice(range.clone())
            .map(|s| (*s).to_vec())
            .unwrap_or_default(),
        CallInput::Bytes(bytes) => bytes.0.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_staking_state() {
        let mut state = StakingState::new();
        let delegator = Address::repeat_byte(0xAA);
        let validator = Address::repeat_byte(0x11);
        let amount = U256::from(1000);

        state.delegate(delegator, validator, amount);
        assert_eq!(state.get_stake(&delegator, &validator), amount);
        assert_eq!(state.total_stake(&validator), amount);

        state.delegate(delegator, validator, amount);
        assert_eq!(state.get_stake(&delegator, &validator), U256::from(2000));

        assert!(state.unbond(delegator, validator, U256::from(500)).is_ok());
        assert_eq!(state.get_stake(&delegator, &validator), U256::from(1500));

        assert!(state.unbond(delegator, validator, U256::from(2000)).is_err());

        let delegator2 = Address::repeat_byte(0xBB);
        state.delegate(delegator2, validator, U256::from(300));
        assert_eq!(state.total_stake(&validator), U256::from(1800));
    }

    #[test]
    fn test_precompile_addresses() {
        assert_eq!(
            BALANCES_ADDR,
            Address::from_slice(&{
                let mut a = [0u8; 20];
                a[18] = 0x08;
                a
            })
        );
        assert_eq!(
            STAKING_ADDR,
            Address::from_slice(&{
                let mut a = [0u8; 20];
                a[18] = 0x08;
                a[19] = 0x01;
                a
            })
        );
    }

    #[test]
    fn test_function_selectors() {
        use alloy_primitives::keccak256;

        assert_eq!(&keccak256(b"balanceOf(address)")[..4], &SEL_BALANCE_OF);
        assert_eq!(
            &keccak256(b"transfer(address,uint256)")[..4],
            &SEL_TRANSFER
        );
        assert_eq!(
            &keccak256(b"delegate(address,uint256)")[..4],
            &SEL_DELEGATE
        );
        assert_eq!(
            &keccak256(b"unbond(address,uint256)")[..4],
            &SEL_UNBOND
        );
        assert_eq!(
            &keccak256(b"getStake(address,address)")[..4],
            &SEL_GET_STAKE
        );
        assert_eq!(
            &keccak256(b"totalStake(address)")[..4],
            &SEL_TOTAL_STAKE
        );
    }
}
