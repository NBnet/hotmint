//! Persistent stores retain their contents across process restarts and read-only opens.
use hotmint_consensus::{evidence_store::EvidenceStore, store::BlockStore};
use hotmint_storage::{
    block_store::VsdbBlockStore, consensus_state::PersistentConsensusState,
    evidence_store::PersistentEvidenceStore,
};
use hotmint_types::{
    AggregateSignature, Block, BlockHash, EndBlockResponse, Epoch, EpochNumber, Height,
    QuorumCertificate, Signature, ValidatorId, ValidatorSet, ViewNumber,
    evidence::EquivocationProof, vote::VoteType,
};
use std::{path::Path, process::Command};

fn block(n: u64) -> Block {
    let hash = |x: u64| BlockHash(*blake3::hash(&x.to_le_bytes()).as_bytes());
    Block {
        height: Height(n),
        parent_hash: if n == 1 {
            Block::genesis().hash
        } else {
            hash(n - 1)
        },
        view: ViewNumber(n),
        proposer: ValidatorId(n % 4),
        timestamp: n * 100,
        payload: n.to_le_bytes().to_vec(),
        app_hash: hash(n + 1000),
        evidence: vec![],
        hash: hash(n),
    }
}
fn qc(b: &Block) -> QuorumCertificate {
    QuorumCertificate {
        block_hash: b.hash,
        view: b.view,
        aggregate_signature: AggregateSignature::new(4),
        epoch: EpochNumber(0),
    }
}
fn evidence(n: u64) -> EquivocationProof {
    EquivocationProof {
        validator: ValidatorId(n),
        view: ViewNumber(n),
        vote_type: VoteType::Vote,
        epoch: EpochNumber(0),
        block_hash_a: BlockHash([1; 32]),
        signature_a: Signature(vec![1; 64]),
        extension_a: None,
        block_hash_b: BlockHash([2; 32]),
        signature_b: Signature(vec![2; 64]),
        extension_b: None,
    }
}
fn phase(dir: &Path, phase: &str) {
    let options = if phase == "read-only" {
        vsdb::VsdbOptions::read_only(dir.join("db"))
    } else {
        vsdb::VsdbOptions::new(dir.join("db"))
    };
    vsdb::vsdb_configure(options).unwrap();
    let mut blocks = VsdbBlockStore::open(dir).unwrap();
    let mut state = PersistentConsensusState::open(dir).unwrap();
    let mut proofs = PersistentEvidenceStore::open(dir).unwrap();
    if phase == "seed" || phase == "append" {
        let (lo, hi) = if phase == "seed" {
            (1, 128)
        } else {
            (129, 129)
        };
        for n in lo..=hi {
            let b = block(n);
            blocks.put_block(b.clone());
            blocks.put_commit_qc(Height(n), qc(&b));
            blocks.put_tx_index(b.hash.0, Height(n), (n % 8) as u32);
            blocks.put_block_results(
                Height(n),
                EndBlockResponse {
                    app_hash: b.app_hash,
                    ..Default::default()
                },
            );
        }
        state.save_current_view(ViewNumber(hi + 1));
        state.save_locked_qc(&qc(&block(hi)));
        state.save_highest_qc(&qc(&block(hi)));
        state.save_last_committed_height(Height(hi));
        state.save_last_app_hash(block(hi).app_hash);
        state.save_current_epoch(&Epoch::genesis(ValidatorSet::new(vec![])));
        state.save_previous_epoch(Some(&Epoch::genesis(ValidatorSet::new(vec![]))));
        state.save_pending_epoch(Some(&Epoch::genesis(ValidatorSet::new(vec![]))));
        if phase == "seed" {
            proofs.put_evidence(evidence(1));
            proofs.put_evidence(evidence(2));
        } else {
            proofs.mark_committed(ViewNumber(1), ValidatorId(1));
            proofs.put_evidence(evidence(3));
            state.save_pending_epoch(None);
        }
        blocks.flush();
        state.flush();
        proofs.flush();
    }
    let hi = if phase == "append" || phase == "read-appended" {
        129
    } else {
        128
    };
    assert_eq!(blocks.tip_height(), Height(hi));
    for n in 1..=hi {
        let b = block(n);
        assert_eq!(
            postcard::to_allocvec(&blocks.get_block(&b.hash).unwrap()).unwrap(),
            postcard::to_allocvec(&b).unwrap()
        );
        assert_eq!(blocks.get_block_by_height(Height(n)).unwrap().hash, b.hash);
        assert_eq!(blocks.get_commit_qc(Height(n)).unwrap().block_hash, b.hash);
        assert_eq!(
            blocks.get_tx_location(&b.hash.0),
            Some((Height(n), (n % 8) as u32))
        );
        assert_eq!(
            blocks.get_block_results(Height(n)).unwrap().app_hash,
            b.app_hash
        );
    }
    assert_eq!(blocks.get_blocks_in_range(Height(17), Height(31)).len(), 15);
    assert_eq!(state.load_current_view(), Some(ViewNumber(hi + 1)));
    assert_eq!(state.load_locked_qc().unwrap().block_hash, block(hi).hash);
    assert_eq!(state.load_highest_qc().unwrap().block_hash, block(hi).hash);
    assert_eq!(state.load_last_committed_height(), Some(Height(hi)));
    assert_eq!(state.load_last_app_hash(), Some(block(hi).app_hash));
    assert_eq!(state.load_current_epoch().unwrap().number, EpochNumber(0));
    assert!(state.load_previous_epoch().is_some());
    assert_eq!(state.load_pending_epoch().is_some(), hi == 128);
    let mut pending: Vec<_> = proofs.get_pending().iter().map(|p| p.validator.0).collect();
    pending.sort();
    assert_eq!(pending, if hi == 128 { vec![1, 2] } else { vec![2, 3] });
    println!("PROBE phase={phase} height={hi}");
    // Simulate an exit that skips Rust destructors after explicit durability.
    std::process::exit(0);
}
#[test]
fn storage_process_boundary() {
    if let Ok(action) = std::env::var("HOTMINT_VSDB_PHASE") {
        phase(
            Path::new(&std::env::var("HOTMINT_VSDB_DIR").unwrap()),
            &action,
        );
    }
    let dir = tempfile::tempdir().unwrap();
    for action in ["seed", "read", "read-only", "append", "read-appended"] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage_process_boundary", "--nocapture"])
            .env("HOTMINT_VSDB_PHASE", action)
            .env("HOTMINT_VSDB_DIR", dir.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{action}: {}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
