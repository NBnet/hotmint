use utxo_chain::{OutPoint, TxInput, TxOutput, UtxoTx, utxo_state::UtxoState};
use vsdb::{MptCalc, MptProof, SmtCalc, SmtProof};

#[test]
fn serde_json_roundtrips_both_proof_kinds() {
    let mut mpt = MptCalc::new();
    let mut smt = SmtCalc::new();
    mpt.insert(b"key", b"value").unwrap();
    smt.insert(b"key", b"value").unwrap();
    let mpt_root: [u8; 32] = mpt.root_hash().unwrap().try_into().unwrap();
    let smt_root: [u8; 32] = smt.root_hash().unwrap().try_into().unwrap();
    for key in [b"key".as_slice(), b"missing"] {
        let mpt = mpt.prove(key).unwrap();
        let smt = smt.prove(key).unwrap();
        let mpt: MptProof = serde_json::from_slice(&serde_json::to_vec(&mpt).unwrap()).unwrap();
        let smt: SmtProof = serde_json::from_slice(&serde_json::to_vec(&smt).unwrap()).unwrap();
        assert!(MptCalc::verify_proof(&mpt_root, key, &mpt).unwrap());
        assert!(SmtCalc::verify_proof(&smt_root, key, &smt).unwrap());
        let expected = (key == b"key").then_some(b"value".as_slice());
        assert_eq!(mpt.value(), expected);
        assert_eq!(smt.value(), expected);
    }
}

#[test]
fn proofs_follow_uncommitted_changes_without_a_root_query() {
    let mut state = UtxoState::new();
    let point = OutPoint {
        txid: [9; 32],
        vout: 0,
    };
    let output = TxOutput {
        value: 2,
        pubkey_hash: [7; 32],
    };
    state.insert_genesis_utxo(point.txid, point.vout, output.clone());
    let proof = state.prove_utxo(&point).unwrap();
    assert_eq!(
        proof.value(),
        Some(postcard::to_allocvec(&output).unwrap().as_slice())
    );
    let root: [u8; 32] = state.utxo_root().unwrap().try_into().unwrap();
    assert!(SmtCalc::verify_proof(&root, &point.to_key(), &proof).unwrap());
    state.commit().unwrap();

    state.apply_tx(
        &UtxoTx {
            inputs: vec![TxInput {
                prev_out: point.clone(),
                signature: vec![],
                pubkey: [0; 32],
            }],
            outputs: vec![output],
        },
        &[10; 32],
    );
    let proof = state.prove_utxo(&point).unwrap();
    assert!(proof.value().is_none());
    let root: [u8; 32] = state.utxo_root().unwrap().try_into().unwrap();
    assert!(SmtCalc::verify_proof(&root, &point.to_key(), &proof).unwrap());
}

#[test]
fn balance_includes_outputs_beyond_the_largest_page() {
    let mut state = UtxoState::new();
    let owner = [7; 32];
    let txid = [9; 32];
    for vout in 0..=u32::from(u16::MAX) {
        state.insert_genesis_utxo(
            txid,
            vout,
            TxOutput {
                value: 1,
                pubkey_hash: owner,
            },
        );
    }
    assert_eq!(state.get_balance(&owner), 65_536);
    assert_eq!(
        state.get_utxos_by_address(&owner, 1, u16::MAX),
        vec![OutPoint {
            txid,
            vout: u32::from(u16::MAX),
        }]
    );
    assert_eq!(state.get_balance(&[8; 32]), 0);
}
