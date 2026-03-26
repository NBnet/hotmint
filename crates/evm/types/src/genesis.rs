use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Genesis account allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisAlloc {
    pub balance: U256,
    #[serde(default)]
    pub nonce: u64,
    #[serde(default, with = "hex_bytes")]
    pub code: Vec<u8>,
    #[serde(default)]
    pub storage: BTreeMap<U256, U256>,
}

/// EVM genesis document (loaded from JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmGenesis {
    pub chain_id: u64,
    pub alloc: BTreeMap<Address, GenesisAlloc>,
    #[serde(default = "default_gas_limit")]
    pub gas_limit: u64,
    #[serde(default = "default_base_fee")]
    pub base_fee_per_gas: u64,
    #[serde(default)]
    pub coinbase: Address,
    #[serde(default)]
    pub timestamp: u64,
}

fn default_gas_limit() -> u64 {
    30_000_000
}

fn default_base_fee() -> u64 {
    1_000_000_000 // 1 gwei
}

impl EvmGenesis {
    pub fn load(path: &std::path::Path) -> ruc::Result<Self> {
        let data = std::fs::read_to_string(path).map_err(|e| ruc::eg!(e))?;
        serde_json::from_str(&data).map_err(|e| ruc::eg!(e))
    }
}

mod hex_bytes {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_string = format!("0x{}", hex::encode(bytes));
        serializer.serialize_str(&hex_string)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        hex::decode(s).map_err(serde::de::Error::custom)
    }

    mod hex {
        pub fn encode(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }

        pub fn decode(s: &str) -> Result<Vec<u8>, String> {
            if s.len() % 2 != 0 {
                return Err("odd length hex string".to_string());
            }
            (0..s.len())
                .step_by(2)
                .map(|i| {
                    u8::from_str_radix(&s[i..i + 2], 16)
                        .map_err(|e| format!("invalid hex: {e}"))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_deserialize() {
        let json = r#"{
            "chain_id": 1337,
            "alloc": {
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": {
                    "balance": "0xde0b6b3a7640000"
                }
            }
        }"#;
        let genesis: EvmGenesis = serde_json::from_str(json).unwrap();
        assert_eq!(genesis.chain_id, 1337);
        assert_eq!(genesis.alloc.len(), 1);
        assert_eq!(genesis.gas_limit, 30_000_000);
        assert_eq!(genesis.base_fee_per_gas, 1_000_000_000);
    }
}
