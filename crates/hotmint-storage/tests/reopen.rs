use hotmint_consensus::{evidence_store::EvidenceStore, store::BlockStore};
use hotmint_storage::{
    block_store::VsdbBlockStore, consensus_state::PersistentConsensusState,
    evidence_store::PersistentEvidenceStore,
};
use hotmint_types::{
    Block, BlockHash, Epoch, EpochNumber, Height, Signature, ValidatorId, ValidatorSet, ViewNumber,
    evidence::EquivocationProof, vote::VoteType,
};

#[test]
fn stores_reopen_after_creation_inside_an_ambient_namespace() {
    let dir = tempfile::tempdir().unwrap();
    vsdb::vsdb_set_base_dir(dir.path().join("db")).unwrap();
    let namespace = vsdb::Namespace::create().unwrap();
    namespace.scope(|| {
        let mut blocks = VsdbBlockStore::open(dir.path()).unwrap();
        blocks.put_tx_index([7; 32], Height(3), 2);
        blocks.flush();

        let mut state = PersistentConsensusState::open(dir.path()).unwrap();
        state.save_current_view(ViewNumber(42));
        state.save_previous_epoch(Some(&Epoch::genesis(ValidatorSet::new(vec![]))));
        state.save_last_committed_height(Height(3));
        state.save_last_app_hash(BlockHash([8; 32]));
        state.flush();

        let mut evidence = PersistentEvidenceStore::open(dir.path()).unwrap();
        evidence.put_evidence(EquivocationProof {
            validator: ValidatorId(1),
            view: ViewNumber(2),
            vote_type: VoteType::Vote,
            epoch: EpochNumber(0),
            block_hash_a: BlockHash([1; 32]),
            signature_a: Signature(vec![1; 64]),
            extension_a: None,
            block_hash_b: BlockHash([2; 32]),
            signature_b: Signature(vec![2; 64]),
            extension_b: None,
        });
        evidence.flush();
    });

    let blocks = VsdbBlockStore::open(dir.path()).unwrap();
    assert_eq!(
        blocks.get_block_by_height(Height::GENESIS).unwrap().hash,
        Block::genesis().hash
    );
    assert_eq!(blocks.get_tx_location(&[7; 32]), Some((Height(3), 2)));
    let mut state = PersistentConsensusState::open(dir.path()).unwrap();
    assert_eq!(state.load_current_view(), Some(ViewNumber(42)));
    assert_eq!(state.load_last_committed_height(), Some(Height(3)));
    assert_eq!(state.load_last_app_hash(), Some(BlockHash([8; 32])));
    assert_eq!(
        state.load_previous_epoch().unwrap().number,
        EpochNumber::GENESIS
    );
    state.save_previous_epoch(None);
    state.flush();
    assert!(
        PersistentConsensusState::open(dir.path())
            .unwrap()
            .load_previous_epoch()
            .is_none()
    );
    let evidence = PersistentEvidenceStore::open(dir.path()).unwrap();
    let pending = evidence.get_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].validator, ValidatorId(1));
}
