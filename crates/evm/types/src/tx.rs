use alloy_consensus::transaction::SignerRecoverable;
use alloy_consensus::{Transaction, TxEnvelope};
use alloy_eips::Decodable2718;
use alloy_primitives::{Address, B256, U256};

/// A decoded and sender-recovered Ethereum transaction.
#[derive(Debug, Clone)]
pub struct VerifiedTx {
    /// The raw RLP-encoded transaction bytes (as received from the network).
    pub raw: Vec<u8>,
    /// The decoded transaction envelope.
    pub envelope: TxEnvelope,
    /// Recovered sender address.
    pub sender: Address,
    /// Transaction hash.
    pub tx_hash: B256,
}

/// Error returned when transaction decoding or verification fails.
#[derive(Debug)]
pub enum TxError {
    RlpDecode(String),
    SignatureRecovery(String),
    ChainIdMismatch { expected: u64, got: Option<u64> },
    NonceTooLow { expected: u64, got: u64 },
    InsufficientBalance { required: U256, available: U256 },
    IntrinsicGasTooLow { required: u64, got: u64 },
    GasLimitExceedsBlock { tx_gas: u64, block_limit: u64 },
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RlpDecode(e) => write!(f, "RLP decode: {e}"),
            Self::SignatureRecovery(e) => write!(f, "signature recovery: {e}"),
            Self::ChainIdMismatch { expected, got } => {
                write!(f, "chain_id mismatch: expected {expected}, got {got:?}")
            }
            Self::NonceTooLow { expected, got } => {
                write!(f, "nonce too low: expected >= {expected}, got {got}")
            }
            Self::InsufficientBalance {
                required,
                available,
            } => {
                write!(f, "insufficient balance: need {required}, have {available}")
            }
            Self::IntrinsicGasTooLow { required, got } => {
                write!(f, "intrinsic gas too low: need {required}, got {got}")
            }
            Self::GasLimitExceedsBlock {
                tx_gas,
                block_limit,
            } => {
                write!(f, "gas limit {tx_gas} exceeds block limit {block_limit}")
            }
        }
    }
}

impl std::error::Error for TxError {}

/// Decode raw Ethereum transaction bytes and recover the sender.
///
/// Accepts EIP-2718 typed transaction envelopes (type 1/2/4)
/// and legacy (untyped) RLP-encoded transactions.
pub fn decode_and_recover(raw: &[u8]) -> Result<VerifiedTx, TxError> {
    let envelope =
        TxEnvelope::decode_2718(&mut &raw[..]).map_err(|e| TxError::RlpDecode(e.to_string()))?;

    let sender = envelope
        .recover_signer()
        .map_err(|e| TxError::SignatureRecovery(format!("{e:?}")))?;

    let tx_hash = *envelope.tx_hash();

    Ok(VerifiedTx {
        raw: raw.to_vec(),
        envelope,
        sender,
        tx_hash,
    })
}

/// Validate a decoded transaction against chain state.
pub fn validate_tx(
    tx: &VerifiedTx,
    chain_id: u64,
    account_nonce: u64,
    account_balance: U256,
    block_gas_limit: u64,
    base_fee: u64,
) -> Result<(), TxError> {
    // Chain ID check.
    let tx_chain_id = tx.envelope.chain_id();
    if let Some(id) = tx_chain_id {
        if id != chain_id {
            return Err(TxError::ChainIdMismatch {
                expected: chain_id,
                got: Some(id),
            });
        }
    }

    // Nonce check.
    let tx_nonce = tx.envelope.nonce();
    if tx_nonce < account_nonce {
        return Err(TxError::NonceTooLow {
            expected: account_nonce,
            got: tx_nonce,
        });
    }

    // Gas limit vs block limit.
    let tx_gas = tx.envelope.gas_limit();
    if tx_gas > block_gas_limit {
        return Err(TxError::GasLimitExceedsBlock {
            tx_gas,
            block_limit: block_gas_limit,
        });
    }

    // Intrinsic gas check.
    let intrinsic = intrinsic_gas(&tx.envelope);
    if tx_gas < intrinsic {
        return Err(TxError::IntrinsicGasTooLow {
            required: intrinsic,
            got: tx_gas,
        });
    }

    // Upfront cost check.
    let max_fee = tx.envelope.max_fee_per_gas().max(base_fee as u128);
    let upfront = U256::from(tx_gas) * U256::from(max_fee) + tx.envelope.value();
    if upfront > account_balance {
        return Err(TxError::InsufficientBalance {
            required: upfront,
            available: account_balance,
        });
    }

    Ok(())
}

/// Calculate the intrinsic gas cost of a transaction.
fn intrinsic_gas(tx: &TxEnvelope) -> u64 {
    let base_cost: u64 = if tx.to().is_some() { 21_000 } else { 53_000 };
    let input = tx.input();
    let data_cost: u64 = input.iter().fold(0u64, |acc, &byte| {
        acc.saturating_add(if byte == 0 { 4 } else { 16 })
    });
    base_cost.saturating_add(data_cost)
}

/// Compute the effective gas tip for a transaction given a base fee.
pub fn effective_gas_tip(tx: &TxEnvelope, base_fee: u64) -> u64 {
    let max_fee = tx.max_fee_per_gas() as u64;
    let max_priority = tx.max_priority_fee_per_gas().unwrap_or(0) as u64;
    let tip = max_fee.saturating_sub(base_fee);
    tip.min(max_priority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxLegacy};
    use alloy_eips::Encodable2718;
    use alloy_primitives::{Bytes, Signature, TxKind};

    fn make_signed_legacy_tx() -> (Vec<u8>, Address) {
        let signing_key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let address = alloy_primitives::Address::from_private_key(&signing_key);

        let mut tx = TxLegacy {
            chain_id: Some(1337),
            nonce: 0,
            gas_price: 1_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(Address::repeat_byte(0xBB)),
            value: U256::from(1_000_000_000_000_000_000u128),
            input: Bytes::new(),
        };

        let sig_hash = tx.signature_hash();
        let (sig, recid) = signing_key
            .sign_prehash_recoverable(sig_hash.as_slice())
            .expect("sign");
        let signature = Signature::from_signature_and_parity(sig, recid.is_y_odd());

        let signed = tx.into_signed(signature);
        let envelope = TxEnvelope::Legacy(signed);
        let mut buf = Vec::new();
        envelope.encode_2718(&mut buf);

        (buf, address)
    }

    #[test]
    fn test_decode_and_recover_legacy() {
        let (raw, expected_sender) = make_signed_legacy_tx();
        let verified = decode_and_recover(&raw).expect("decode should succeed");
        assert_eq!(verified.sender, expected_sender);
        assert_eq!(verified.envelope.chain_id(), Some(1337));
        assert_eq!(verified.envelope.nonce(), 0);
    }

    #[test]
    fn test_validate_tx_ok() {
        let (raw, _) = make_signed_legacy_tx();
        let verified = decode_and_recover(&raw).unwrap();
        let result = validate_tx(
            &verified,
            1337,
            0,
            U256::from(100_000_000_000_000_000_000u128),
            30_000_000,
            1_000_000_000,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_wrong_chain_id() {
        let (raw, _) = make_signed_legacy_tx();
        let verified = decode_and_recover(&raw).unwrap();
        let result = validate_tx(
            &verified,
            9999,
            0,
            U256::from(100_000_000_000_000_000_000u128),
            30_000_000,
            1_000_000_000,
        );
        assert!(matches!(result, Err(TxError::ChainIdMismatch { .. })));
    }

    #[test]
    fn test_validate_tx_nonce_too_low() {
        let (raw, _) = make_signed_legacy_tx();
        let verified = decode_and_recover(&raw).unwrap();
        let result = validate_tx(
            &verified,
            1337,
            5, // account nonce is 5, tx nonce is 0
            U256::from(100_000_000_000_000_000_000u128),
            30_000_000,
            1_000_000_000,
        );
        assert!(matches!(result, Err(TxError::NonceTooLow { .. })));
    }

    #[test]
    fn test_validate_tx_insufficient_balance() {
        let (raw, _) = make_signed_legacy_tx();
        let verified = decode_and_recover(&raw).unwrap();
        let result = validate_tx(
            &verified,
            1337,
            0,
            U256::from(1u64), // almost no balance
            30_000_000,
            1_000_000_000,
        );
        assert!(matches!(result, Err(TxError::InsufficientBalance { .. })));
    }

    #[test]
    fn test_decode_garbage() {
        let result = decode_and_recover(&[0xFF, 0x01, 0x02]);
        assert!(matches!(result, Err(TxError::RlpDecode(_))));
    }

    #[test]
    fn test_decode_eip1559_tx() {
        use alloy_consensus::TxEip1559;

        let signing_key = k256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        let expected = alloy_primitives::Address::from_private_key(&signing_key);

        let mut tx = TxEip1559 {
            chain_id: 1337,
            nonce: 42,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(Address::repeat_byte(0xCC)),
            value: U256::from(500_000_000_000_000_000u128),
            input: Bytes::new(),
            access_list: Default::default(),
        };

        let sig_hash = tx.signature_hash();
        let (sig, recid) = signing_key
            .sign_prehash_recoverable(sig_hash.as_slice())
            .expect("sign");
        let signature = Signature::from_signature_and_parity(sig, recid.is_y_odd());

        let signed = tx.into_signed(signature);
        let envelope = TxEnvelope::Eip1559(signed);
        let mut buf = Vec::new();
        envelope.encode_2718(&mut buf);

        let verified = decode_and_recover(&buf).expect("decode");
        assert_eq!(verified.sender, expected);
        assert_eq!(verified.envelope.chain_id(), Some(1337));
        assert_eq!(verified.envelope.nonce(), 42);
        assert_eq!(verified.envelope.max_fee_per_gas(), 30_000_000_000);
        assert_eq!(
            verified.envelope.max_priority_fee_per_gas(),
            Some(1_000_000_000)
        );

        // Effective tip with base_fee = 10 gwei
        let tip = effective_gas_tip(&verified.envelope, 10_000_000_000);
        assert_eq!(tip, 1_000_000_000);
    }
}
