use serde::{Deserialize, Serialize};

/// Compatibility profile for EVM chain behavior.
///
/// `Modern` (default): Only supports modern Ethereum semantics.
/// `Legacy`: Enables additional compatibility for historical behaviors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompatProfile {
    #[default]
    Modern,
    Legacy,
}

/// EVM chain runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmChainConfig {
    pub chain_id: u64,
    pub block_gas_limit: u64,
    pub base_fee_per_gas: u64,
    #[serde(default)]
    pub compat_profile: CompatProfile,
    #[serde(default = "default_min_base_fee")]
    pub min_base_fee: u64,
    #[serde(default = "default_base_fee_change_denom")]
    pub base_fee_change_denominator: u64,
    #[serde(default = "default_elasticity_multiplier")]
    pub elasticity_multiplier: u64,
}

fn default_min_base_fee() -> u64 {
    1_000_000_000 // 1 gwei
}

fn default_base_fee_change_denom() -> u64 {
    8
}

fn default_elasticity_multiplier() -> u64 {
    2
}

impl Default for EvmChainConfig {
    fn default() -> Self {
        Self {
            chain_id: 1337,
            block_gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000,
            compat_profile: CompatProfile::default(),
            min_base_fee: default_min_base_fee(),
            base_fee_change_denominator: default_base_fee_change_denom(),
            elasticity_multiplier: default_elasticity_multiplier(),
        }
    }
}
