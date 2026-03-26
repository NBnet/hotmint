use alloy_primitives::{Address, B256, Bytes, U256};
use serde::{Deserialize, Serialize};

/// An EVM execution log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
}

/// An EVM transaction receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmReceipt {
    /// Transaction hash.
    pub tx_hash: B256,
    /// Index of the transaction within the block.
    pub tx_index: u64,
    /// Block hash (populated after block finalization).
    pub block_hash: B256,
    /// Block number.
    pub block_number: u64,
    /// Sender address.
    pub from: Address,
    /// Recipient address (None for contract creation).
    pub to: Option<Address>,
    /// Cumulative gas used up to and including this transaction.
    pub cumulative_gas_used: u64,
    /// Gas used by this individual transaction.
    pub gas_used: u64,
    /// Contract address created, if any.
    pub contract_address: Option<Address>,
    /// Logs emitted by this transaction.
    pub logs: Vec<EvmLog>,
    /// Logs bloom filter (2048 bits = 256 bytes).
    #[serde(with = "bloom_serde")]
    pub logs_bloom: [u8; 256],
    /// Status: 1 = success, 0 = revert.
    pub status: u8,
    /// Effective gas price paid.
    pub effective_gas_price: U256,
}

mod bloom_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bloom: &[u8; 256], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex: String = bloom.iter().map(|b| format!("{b:02x}")).collect();
        serializer.serialize_str(&format!("0x{hex}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 256], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        if s.len() != 512 {
            return Err(serde::de::Error::custom("bloom must be 256 bytes hex"));
        }
        let mut arr = [0u8; 256];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex_str = std::str::from_utf8(chunk).map_err(serde::de::Error::custom)?;
            arr[i] = u8::from_str_radix(hex_str, 16).map_err(serde::de::Error::custom)?;
        }
        Ok(arr)
    }
}
