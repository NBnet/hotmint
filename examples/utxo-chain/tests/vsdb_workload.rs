//! UTXO state transitions checked against an independent in-memory model.
use std::{collections::BTreeMap, time::Instant};
use utxo_chain::{OutPoint, TxInput, TxOutput, UtxoTx, utxo_state::UtxoState};
use vsdb::SmtCalc;

fn outpoint(n: u64) -> OutPoint {
    OutPoint {
        txid: *blake3::hash(&n.to_le_bytes()).as_bytes(),
        vout: 0,
    }
}

fn spend(old: OutPoint, owner: u8) -> UtxoTx {
    UtxoTx {
        inputs: vec![TxInput {
            prev_out: old,
            signature: vec![],
            pubkey: [0; 32],
        }],
        outputs: vec![TxOutput {
            value: 100,
            pubkey_hash: [owner; 32],
        }],
    }
}

#[test]
fn utxo_spends_pages_roots_and_proofs_match_an_independent_model() {
    let mut state = UtxoState::new();
    let mut expected = BTreeMap::new();
    for n in 0..128 {
        let point = outpoint(n);
        let output = TxOutput {
            value: 100,
            pubkey_hash: [(n % 4) as u8; 32],
        };
        state.insert_genesis_utxo(point.txid, point.vout, output.clone());
        expected.insert(point.to_key(), output);
    }
    for block in 0..9 {
        if block > 0 {
            for n in (block - 1) * 8..block * 8 {
                let old = outpoint(n);
                let new = outpoint(1000 + n);
                let tx = spend(old.clone(), ((n + 1) % 4) as u8);
                state.apply_tx(&tx, &new.txid);
                expected.remove(&old.to_key());
                expected.insert(new.to_key(), tx.outputs[0].clone());
                assert!(state.get_utxo(&old).is_none());
            }
        }
        // Check the dirty state too, then the committed state.
        let mut oracle = SmtCalc::from_entries(
            expected
                .iter()
                .map(|(k, v)| (k.to_vec(), postcard::to_allocvec(v).unwrap())),
        )
        .unwrap();
        let expected_root = oracle.root_hash().unwrap();
        assert_eq!(state.utxo_root().unwrap(), expected_root);
        state.commit().unwrap();
        let root: [u8; 32] = state.utxo_root().unwrap().try_into().unwrap();
        assert_eq!(root.as_slice(), expected_root);
        assert_eq!(state.total_supply(), 12_800);
        for owner in 0..4 {
            let keys: Vec<_> = expected
                .iter()
                .filter(|(_, v)| v.pubkey_hash == [owner; 32])
                .map(|(k, _)| *k)
                .collect();
            assert_eq!(state.get_balance(&[owner; 32]), keys.len() as u64 * 100);
            for (page, chunk) in keys.chunks(7).enumerate() {
                assert_eq!(
                    state
                        .get_utxos_by_address(&[owner; 32], page as u32, 7)
                        .iter()
                        .map(OutPoint::to_key)
                        .collect::<Vec<_>>(),
                    chunk
                );
            }
            assert!(
                state
                    .get_utxos_by_address(&[owner; 32], keys.len().div_ceil(7) as u32, 7)
                    .is_empty()
            );
        }
        for n in [0, 63, 127, 1000, 1063, 9999] {
            let key = outpoint(n).to_key();
            let proof = state.prove_utxo(&OutPoint::from_key(&key)).unwrap();
            assert!(SmtCalc::verify_proof(&root, &key, &proof).unwrap());
            assert_eq!(
                proof.value(),
                expected
                    .get(&key)
                    .map(|v| postcard::to_allocvec(v).unwrap())
                    .as_deref()
            );
        }
    }
}

#[test]
#[ignore = "explicit release-mode performance probe"]
fn utxo_block_cost() {
    let sizes: Vec<u64> = std::env::var("HOTMINT_UTXO_SIZES")
        .unwrap_or_else(|_| "128,2048,8192".into())
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    for size in sizes {
        let mut state = UtxoState::new();
        let started = Instant::now();
        for n in 0..size {
            let point = outpoint(n);
            state.insert_genesis_utxo(
                point.txid,
                0,
                TxOutput {
                    value: 100,
                    pubkey_hash: [1; 32],
                },
            );
        }
        state.commit().unwrap();
        state.utxo_root().unwrap();
        let setup_us = started.elapsed().as_micros();
        let mut mutation = Vec::new();
        let mut commit = Vec::new();
        let mut root = Vec::new();
        for n in 0..20 {
            let started = Instant::now();
            state.apply_tx(&spend(outpoint(n), 2), &outpoint(size + n).txid);
            mutation.push(started.elapsed().as_micros());
            let started = Instant::now();
            state.commit().unwrap();
            commit.push(started.elapsed().as_micros());
            let started = Instant::now();
            let hash: [u8; 32] = state.utxo_root().unwrap().try_into().unwrap();
            root.push(started.elapsed().as_micros());
            let point = outpoint(size + n);
            assert!(
                SmtCalc::verify_proof(&hash, &point.to_key(), &state.prove_utxo(&point).unwrap())
                    .unwrap()
            );
        }
        println!(
            "PERF {}",
            serde_json::json!({"rows":size,"setup_us":setup_us,"mutation_us":mutation,"commit_us":commit,"root_us":root})
        );
    }
}
