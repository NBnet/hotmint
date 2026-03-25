use ruc::*;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::application::Application;
use crate::commit::{CommitResult, try_commit};
use crate::evidence_store::EvidenceStore;
use crate::leader;
use crate::liveness::{LivenessTracker, OfflineEvidence};
use crate::network::NetworkSink;
use crate::pacemaker::{Pacemaker, PacemakerConfig};
use crate::state::{ConsensusState, ViewStep};
use crate::store::BlockStore;
use crate::view_protocol::{self, ViewEntryTrigger};
use crate::vote_collector::VoteCollector;

use hotmint_types::epoch::Epoch;
use hotmint_types::vote::VoteType;
use hotmint_types::*;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Shared block store type used by the engine, RPC, and sync responder.
pub type SharedBlockStore = Arc<parking_lot::RwLock<Box<dyn BlockStore>>>;

/// Trait for persisting critical consensus state across restarts.
pub trait StatePersistence: Send {
    fn save_current_view(&mut self, view: ViewNumber);
    fn save_locked_qc(&mut self, qc: &QuorumCertificate);
    fn save_highest_qc(&mut self, qc: &QuorumCertificate);
    fn save_last_committed_height(&mut self, height: Height);
    fn save_current_epoch(&mut self, epoch: &Epoch);
    fn save_last_app_hash(&mut self, hash: BlockHash);
    fn flush(&self);
}

/// Write-Ahead Log trait for commit crash recovery.
pub trait Wal: Send {
    /// Log intent to commit blocks up to `target_height`. Must fsync before returning.
    fn log_commit_intent(&mut self, target_height: Height) -> std::io::Result<()>;
    /// Log that commit succeeded. May truncate the WAL.
    fn log_commit_done(&mut self, target_height: Height) -> std::io::Result<()>;
}

pub struct ConsensusEngine {
    state: ConsensusState,
    store: SharedBlockStore,
    network: Box<dyn NetworkSink>,
    app: Box<dyn Application>,
    signer: Box<dyn Signer>,
    verifier: Box<dyn Verifier>,
    vote_collector: VoteCollector,
    pacemaker: Pacemaker,
    pacemaker_config: PacemakerConfig,
    msg_rx: mpsc::Receiver<(Option<ValidatorId>, ConsensusMessage)>,
    /// Collected unique status cert senders (for leader, per view)
    status_senders: HashSet<ValidatorId>,
    /// The QC formed in this view's first voting round (used to build DoubleCert)
    current_view_qc: Option<QuorumCertificate>,
    /// Pending epoch transition (set by try_commit, applied in advance_view_to)
    pending_epoch: Option<Epoch>,
    /// Optional state persistence (for crash recovery).
    persistence: Option<Box<dyn StatePersistence>>,
    /// Optional evidence store for persisting equivocation proofs.
    evidence_store: Option<Box<dyn EvidenceStore>>,
    /// Tracks per-validator liveness for offline slashing.
    liveness_tracker: LivenessTracker,
    /// Optional write-ahead log for commit crash recovery.
    wal: Option<Box<dyn Wal>>,
}

/// Configuration for ConsensusEngine.
pub struct EngineConfig {
    pub verifier: Box<dyn Verifier>,
    pub pacemaker: Option<PacemakerConfig>,
    pub persistence: Option<Box<dyn StatePersistence>>,
    pub evidence_store: Option<Box<dyn EvidenceStore>>,
    pub wal: Option<Box<dyn Wal>>,
}

impl EngineConfig {
    /// Create an `EngineConfig` with the given verifier and defaults
    /// (no custom pacemaker, no persistence, no evidence store).
    pub fn new(verifier: Box<dyn Verifier>) -> Self {
        Self {
            verifier,
            pacemaker: None,
            persistence: None,
            evidence_store: None,
            wal: None,
        }
    }

    /// Set a custom pacemaker configuration.
    pub fn with_pacemaker(mut self, pacemaker: PacemakerConfig) -> Self {
        self.pacemaker = Some(pacemaker);
        self
    }

    /// Set a state persistence backend.
    pub fn with_persistence(mut self, persistence: Box<dyn StatePersistence>) -> Self {
        self.persistence = Some(persistence);
        self
    }
}

/// Builder for constructing a `ConsensusEngine` with a fluent API.
///
/// # Example
/// ```rust,ignore
/// let engine = ConsensusEngineBuilder::new()
///     .state(state)
///     .store(store)
///     .network(network)
///     .app(app)
///     .signer(signer)
///     .messages(msg_rx)
///     .verifier(verifier)
///     .build()
///     .expect("all required fields must be set");
/// ```
pub struct ConsensusEngineBuilder {
    state: Option<ConsensusState>,
    store: Option<SharedBlockStore>,
    network: Option<Box<dyn NetworkSink>>,
    app: Option<Box<dyn Application>>,
    signer: Option<Box<dyn Signer>>,
    msg_rx: Option<mpsc::Receiver<(Option<ValidatorId>, ConsensusMessage)>>,
    verifier: Option<Box<dyn Verifier>>,
    pacemaker: Option<PacemakerConfig>,
    persistence: Option<Box<dyn StatePersistence>>,
    evidence_store: Option<Box<dyn EvidenceStore>>,
    wal: Option<Box<dyn Wal>>,
}

impl ConsensusEngineBuilder {
    pub fn new() -> Self {
        Self {
            state: None,
            store: None,
            network: None,
            app: None,
            signer: None,
            msg_rx: None,
            verifier: None,
            pacemaker: None,
            persistence: None,
            evidence_store: None,
            wal: None,
        }
    }

    pub fn state(mut self, state: ConsensusState) -> Self {
        self.state = Some(state);
        self
    }

    pub fn store(mut self, store: SharedBlockStore) -> Self {
        self.store = Some(store);
        self
    }

    pub fn network(mut self, network: Box<dyn NetworkSink>) -> Self {
        self.network = Some(network);
        self
    }

    pub fn app(mut self, app: Box<dyn Application>) -> Self {
        self.app = Some(app);
        self
    }

    pub fn signer(mut self, signer: Box<dyn Signer>) -> Self {
        self.signer = Some(signer);
        self
    }

    pub fn messages(
        mut self,
        msg_rx: mpsc::Receiver<(Option<ValidatorId>, ConsensusMessage)>,
    ) -> Self {
        self.msg_rx = Some(msg_rx);
        self
    }

    pub fn verifier(mut self, verifier: Box<dyn Verifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    pub fn pacemaker(mut self, config: PacemakerConfig) -> Self {
        self.pacemaker = Some(config);
        self
    }

    pub fn persistence(mut self, persistence: Box<dyn StatePersistence>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    pub fn evidence_store(mut self, store: Box<dyn EvidenceStore>) -> Self {
        self.evidence_store = Some(store);
        self
    }

    pub fn wal(mut self, wal: Box<dyn Wal>) -> Self {
        self.wal = Some(wal);
        self
    }

    pub fn build(self) -> ruc::Result<ConsensusEngine> {
        let state = self.state.ok_or_else(|| ruc::eg!("state is required"))?;
        let store = self.store.ok_or_else(|| ruc::eg!("store is required"))?;
        let network = self
            .network
            .ok_or_else(|| ruc::eg!("network is required"))?;
        let app = self.app.ok_or_else(|| ruc::eg!("app is required"))?;
        let signer = self.signer.ok_or_else(|| ruc::eg!("signer is required"))?;
        let msg_rx = self
            .msg_rx
            .ok_or_else(|| ruc::eg!("messages (msg_rx) is required"))?;
        let verifier = self
            .verifier
            .ok_or_else(|| ruc::eg!("verifier is required"))?;

        let config = EngineConfig {
            verifier,
            pacemaker: self.pacemaker,
            persistence: self.persistence,
            evidence_store: self.evidence_store,
            wal: self.wal,
        };

        Ok(ConsensusEngine::new(
            state, store, network, app, signer, msg_rx, config,
        ))
    }
}

impl Default for ConsensusEngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsensusEngine {
    pub fn new(
        state: ConsensusState,
        store: SharedBlockStore,
        network: Box<dyn NetworkSink>,
        app: Box<dyn Application>,
        signer: Box<dyn Signer>,
        msg_rx: mpsc::Receiver<(Option<ValidatorId>, ConsensusMessage)>,
        config: EngineConfig,
    ) -> Self {
        let pc = config.pacemaker.unwrap_or_default();
        Self {
            state,
            store,
            network,
            app,
            signer,
            verifier: config.verifier,
            vote_collector: VoteCollector::new(),
            pacemaker: Pacemaker::with_config(pc.clone()),
            pacemaker_config: pc,
            msg_rx,
            status_senders: HashSet::new(),
            current_view_qc: None,
            pending_epoch: None,
            persistence: config.persistence,
            evidence_store: config.evidence_store,
            liveness_tracker: LivenessTracker::new(),
            wal: config.wal,
        }
    }

    /// Bootstrap and start the event loop.
    /// If persisted state was restored (current_view > 1), skip genesis bootstrap.
    pub async fn run(mut self) {
        // Check application info against consensus state for divergence detection.
        let app_info = self.app.info();
        if app_info.last_block_height.as_u64() > 0
            && app_info.last_block_height != self.state.last_committed_height
        {
            warn!(
                app_height = app_info.last_block_height.as_u64(),
                consensus_height = self.state.last_committed_height.as_u64(),
                "application height differs from consensus state — possible state divergence"
            );
        }

        if self.state.current_view.as_u64() <= 1 {
            self.enter_genesis_view().await;
        } else {
            info!(
                validator = %self.state.validator_id,
                view = %self.state.current_view,
                height = %self.state.last_committed_height,
                "resuming from persisted state"
            );
            self.pacemaker.reset_timer();
        }

        loop {
            let deadline = self.pacemaker.sleep_until_deadline();
            tokio::pin!(deadline);

            tokio::select! {
                Some((sender, msg)) = self.msg_rx.recv() => {
                    if let Err(e) = self.handle_message(sender, msg).await {
                        warn!(validator = %self.state.validator_id, error = %e, "error handling message");
                    }
                }
                _ = &mut deadline => {
                    self.handle_timeout().await;
                }
            }
        }
    }

    async fn enter_genesis_view(&mut self) {
        // Create a synthetic genesis QC so the first leader can propose
        let genesis_qc = QuorumCertificate {
            block_hash: BlockHash::GENESIS,
            view: ViewNumber::GENESIS,
            aggregate_signature: AggregateSignature::new(
                self.state.validator_set.validator_count(),
            ),
            epoch: self.state.current_epoch.number,
        };
        self.state.highest_qc = Some(genesis_qc);

        let view = ViewNumber(1);
        view_protocol::enter_view(
            &mut self.state,
            view,
            ViewEntryTrigger::Genesis,
            self.network.as_ref(),
            self.signer.as_ref(),
        );
        self.pacemaker.reset_timer();

        // If leader of genesis view, propose immediately
        if self.state.is_leader() {
            self.state.step = ViewStep::WaitingForStatus;
            // In genesis, skip status wait — propose directly
            self.try_propose().await;
        }
    }

    fn try_propose(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // Collect pending evidence to embed in the block (C-3).
            let pending_evidence = self
                .evidence_store
                .as_ref()
                .map(|s| s.get_pending())
                .unwrap_or_default();

            // Propose is synchronous; acquire, run, and release the lock before any await.
            let proposed_block = {
                let mut store = self.store.write();
                view_protocol::propose(
                    &mut self.state,
                    store.as_mut(),
                    self.network.as_ref(),
                    self.app.as_ref(),
                    self.signer.as_ref(),
                    pending_evidence,
                )
            }; // lock released here

            match proposed_block {
                Ok(block) => {
                    // Leader votes for its own block
                    self.leader_self_vote(block.hash).await;
                }
                Err(e) => {
                    warn!(
                        validator = %self.state.validator_id,
                        error = %e,
                        "failed to propose"
                    );
                }
            }
        })
    }

    async fn leader_self_vote(&mut self, block_hash: BlockHash) {
        let vote_bytes = Vote::signing_bytes(
            &self.state.chain_id_hash,
            self.state.current_epoch.number,
            self.state.current_view,
            &block_hash,
            VoteType::Vote,
        );
        let signature = self.signer.sign(&vote_bytes);
        let vote = Vote {
            block_hash,
            view: self.state.current_view,
            validator: self.state.validator_id,
            signature,
            vote_type: VoteType::Vote,
            extension: None,
        };
        match self.vote_collector.add_vote(
            &self.state.validator_set,
            vote,
            self.state.current_epoch.number,
        ) {
            Ok(result) => {
                self.handle_equivocation(&result);
                if let Some(qc) = result.qc {
                    self.on_qc_formed(qc).await;
                }
            }
            Err(e) => warn!(error = %e, "failed to add self vote"),
        }
    }
}

/// Verify the per-sender individual signature on a consensus message before relaying.
///
/// Only checks signatures that can be attributed to a single known validator using the
/// provided key map.  Aggregate certificates (TimeoutCert, justify/prepare QCs) are
/// intentionally not fully re-verified here — the receiving engine always does the full
/// check.  Messages whose signing bytes depend on receiver state (StatusCert needs
/// `current_view`) are also allowed through; the engine will reject them if invalid.
///
/// `ordered_validators` is the validator list in round-robin order (same order used by
/// `ValidatorSet::leader_for_view`).  Pass an empty slice to skip the leader-for-view
/// check (e.g., in tests where the set is not available).
///
/// Returns `false` when:
/// - The claimed sender is not in `validator_keys` (unknown/non-validator peer), OR
/// - The individual signature is cryptographically invalid, OR
/// - For Prepare: the sender is not the leader for the certificate's view.
pub fn verify_relay_sender(
    sender: ValidatorId,
    msg: &ConsensusMessage,
    validator_keys: &HashMap<ValidatorId, hotmint_types::crypto::PublicKey>,
    ordered_validators: &[ValidatorId],
    chain_id_hash: &[u8; 32],
    epoch: hotmint_types::epoch::EpochNumber,
) -> bool {
    use hotmint_crypto::Ed25519Verifier;
    use hotmint_types::Verifier;
    use hotmint_types::vote::Vote;
    let verifier = Ed25519Verifier;
    match msg {
        ConsensusMessage::Propose {
            block,
            justify,
            signature,
            ..
        } => {
            let Some(pk) = validator_keys.get(&block.proposer) else {
                return false;
            };
            let bytes =
                crate::view_protocol::proposal_signing_bytes(chain_id_hash, epoch, block, justify);
            Verifier::verify(&verifier, pk, &bytes, signature)
        }
        ConsensusMessage::VoteMsg(vote) | ConsensusMessage::Vote2Msg(vote) => {
            let Some(pk) = validator_keys.get(&vote.validator) else {
                return false;
            };
            let bytes = Vote::signing_bytes(
                chain_id_hash,
                epoch,
                vote.view,
                &vote.block_hash,
                vote.vote_type,
            );
            Verifier::verify(&verifier, pk, &bytes, &vote.signature)
        }
        ConsensusMessage::Prepare {
            certificate,
            signature,
        } => {
            // Prepare is broadcast by the current leader. Verify that the relay
            // sender is the leader for this view, then check the signature.
            if !ordered_validators.is_empty() {
                let n = ordered_validators.len();
                let expected_leader = ordered_validators[certificate.view.as_u64() as usize % n];
                if sender != expected_leader {
                    return false;
                }
            }
            let Some(pk) = validator_keys.get(&sender) else {
                return false;
            };
            let bytes =
                crate::view_protocol::prepare_signing_bytes(chain_id_hash, epoch, certificate);
            Verifier::verify(&verifier, pk, &bytes, signature)
        }
        ConsensusMessage::Wish {
            target_view,
            validator,
            highest_qc,
            signature,
        } => {
            let Some(pk) = validator_keys.get(validator) else {
                return false;
            };
            let bytes = crate::pacemaker::wish_signing_bytes(
                chain_id_hash,
                epoch,
                *target_view,
                highest_qc.as_ref(),
            );
            Verifier::verify(&verifier, pk, &bytes, signature)
        }
        ConsensusMessage::TimeoutCert(tc) => {
            // Full relay verification: verify each signer's signature + quorum check.
            let target_view = ViewNumber(tc.view.as_u64() + 1);
            let n = ordered_validators.len();
            if n == 0 || tc.aggregate_signature.signers.len() != n {
                return false;
            }
            let mut sig_idx = 0usize;
            let mut verified_count = 0usize;
            for (i, &signed) in tc.aggregate_signature.signers.iter().enumerate() {
                if !signed {
                    continue;
                }
                if i >= n {
                    return false;
                }
                let vid = ordered_validators[i];
                let Some(pk) = validator_keys.get(&vid) else {
                    return false;
                };
                let hqc = tc.highest_qcs.get(i).and_then(|h| h.as_ref());
                let bytes =
                    crate::pacemaker::wish_signing_bytes(chain_id_hash, epoch, target_view, hqc);
                if sig_idx >= tc.aggregate_signature.signatures.len() {
                    return false;
                }
                if !Verifier::verify(
                    &verifier,
                    pk,
                    &bytes,
                    &tc.aggregate_signature.signatures[sig_idx],
                ) {
                    return false;
                }
                sig_idx += 1;
                verified_count += 1;
            }
            if sig_idx != tc.aggregate_signature.signatures.len() {
                return false;
            }
            // Quorum check: require > 2/3 of validators (by count, not power —
            // full power-based check is done in engine::verify_message).
            verified_count * 3 > n * 2
        }
        ConsensusMessage::StatusCert {
            validator,
            signature,
            locked_qc,
            ..
        } => {
            // StatusCert signing bytes require current_view which we don't have
            // in the relay context. Verify the sender is a known validator and
            // the signature is over *some* plausible view (the TC view is not
            // available here). The engine does full verification with correct view.
            // At minimum, reject unknown validators.
            let Some(pk) = validator_keys.get(validator) else {
                return false;
            };
            // We cannot construct exact signing bytes without knowing current_view,
            // so we accept from known validators only. The engine's verify_message
            // will do full cryptographic verification.
            let _ = (pk, signature, locked_qc);
            true
        }
        ConsensusMessage::Evidence(_) => {
            // Evidence gossip: accept from any known validator.
            // The engine verifies the proof internally.
            validator_keys.contains_key(&sender)
        }
    }
}

impl ConsensusEngine {
    /// Epoch numbers to try when verifying signatures. During epoch transitions,
    /// some nodes may still be in the previous epoch. We try the current epoch
    /// first, then fall back to epoch - 1 to tolerate the transition window.
    fn verification_epochs(&self) -> [EpochNumber; 2] {
        let cur = self.state.current_epoch.number;
        let prev = if cur.as_u64() > 0 {
            EpochNumber(cur.as_u64() - 1)
        } else {
            cur
        };
        [cur, prev]
    }

    /// Verify the cryptographic signature on an inbound consensus message.
    /// Returns false (and logs a warning) if verification fails.
    /// Messages from past views are skipped (they'll be dropped by handle_message anyway).
    ///
    /// Crypto-heavy paths (aggregate signature verification) are run via
    /// `tokio::task::block_in_place` so the async event loop remains responsive
    /// while Ed25519 batch verification runs on the current OS thread.
    fn verify_message(&self, msg: &ConsensusMessage) -> bool {
        // Skip verification for non-Propose past-view messages — these may have
        // been signed by a previous epoch's validator set. They'll be dropped by
        // view checks. Propose messages are always verified because they may still
        // be stored (for chain continuity in fast-forward).
        let msg_view = match msg {
            ConsensusMessage::Propose { .. } => None, // always verify proposals
            ConsensusMessage::VoteMsg(v) | ConsensusMessage::Vote2Msg(v) => Some(v.view),
            ConsensusMessage::Prepare { certificate, .. } => Some(certificate.view),
            ConsensusMessage::Wish { target_view, .. } => Some(*target_view),
            ConsensusMessage::TimeoutCert(tc) => Some(ViewNumber(tc.view.as_u64() + 1)),
            ConsensusMessage::StatusCert { .. } => None,
            ConsensusMessage::Evidence(_) => None, // always accept evidence
        };
        if let Some(v) = msg_view
            && v < self.state.current_view
        {
            return true; // will be dropped by handler
        }

        let vs = &self.state.validator_set;
        match msg {
            ConsensusMessage::Propose {
                block,
                justify,
                signature,
                ..
            } => {
                let proposer = vs.get(block.proposer);
                let Some(vi) = proposer else {
                    warn!(proposer = %block.proposer, "propose from unknown validator");
                    return false;
                };
                let mut proposal_ok = false;
                for epoch in self.verification_epochs() {
                    let bytes = view_protocol::proposal_signing_bytes(
                        &self.state.chain_id_hash,
                        epoch,
                        block,
                        justify,
                    );
                    if self.verifier.verify(&vi.public_key, &bytes, signature) {
                        proposal_ok = true;
                        break;
                    }
                }
                if !proposal_ok {
                    warn!(proposer = %block.proposer, "invalid proposal signature");
                    return false;
                }
                // Verify justify QC aggregate signature (skip genesis QC which has no signers)
                if justify.aggregate_signature.count() > 0 {
                    let qc_bytes = Vote::signing_bytes(
                        &self.state.chain_id_hash,
                        justify.epoch,
                        justify.view,
                        &justify.block_hash,
                        VoteType::Vote,
                    );
                    if !self
                        .verifier
                        .verify_aggregate(vs, &qc_bytes, &justify.aggregate_signature)
                    {
                        warn!(proposer = %block.proposer, "invalid justify QC aggregate signature");
                        return false;
                    }
                    if justify.epoch == self.state.current_epoch.number
                        && !hotmint_crypto::has_quorum(vs, &justify.aggregate_signature)
                    {
                        warn!(proposer = %block.proposer, "justify QC below quorum threshold");
                        return false;
                    }
                }
                true
            }
            ConsensusMessage::VoteMsg(vote) | ConsensusMessage::Vote2Msg(vote) => {
                let Some(vi) = vs.get(vote.validator) else {
                    warn!(validator = %vote.validator, "vote from unknown validator");
                    return false;
                };
                let mut ok = false;
                for epoch in self.verification_epochs() {
                    let bytes = Vote::signing_bytes(
                        &self.state.chain_id_hash,
                        epoch,
                        vote.view,
                        &vote.block_hash,
                        vote.vote_type,
                    );
                    if self
                        .verifier
                        .verify(&vi.public_key, &bytes, &vote.signature)
                    {
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    warn!(validator = %vote.validator, "invalid vote signature");
                    return false;
                }
                true
            }
            ConsensusMessage::Prepare {
                certificate,
                signature,
            } => {
                // Verify the leader's signature on the prepare message
                let Some(leader) = vs.leader_for_view(certificate.view) else {
                    return false;
                };
                let mut prepare_ok = false;
                for epoch in self.verification_epochs() {
                    let bytes = view_protocol::prepare_signing_bytes(
                        &self.state.chain_id_hash,
                        epoch,
                        certificate,
                    );
                    if self.verifier.verify(&leader.public_key, &bytes, signature) {
                        prepare_ok = true;
                        break;
                    }
                }
                if !prepare_ok {
                    warn!(view = %certificate.view, "invalid prepare signature");
                    return false;
                }
                // Also verify the QC's aggregate signature and quorum
                let qc_bytes = Vote::signing_bytes(
                    &self.state.chain_id_hash,
                    certificate.epoch,
                    certificate.view,
                    &certificate.block_hash,
                    VoteType::Vote,
                );
                if !self
                    .verifier
                    .verify_aggregate(vs, &qc_bytes, &certificate.aggregate_signature)
                {
                    warn!(view = %certificate.view, "invalid QC aggregate signature");
                    return false;
                }
                if certificate.epoch == self.state.current_epoch.number
                    && !hotmint_crypto::has_quorum(vs, &certificate.aggregate_signature)
                {
                    warn!(view = %certificate.view, "Prepare QC below quorum threshold");
                    return false;
                }
                true
            }
            ConsensusMessage::Wish {
                target_view,
                validator,
                highest_qc,
                signature,
            } => {
                let Some(vi) = vs.get(*validator) else {
                    warn!(validator = %validator, "wish from unknown validator");
                    return false;
                };
                // Signing bytes bind both target_view and highest_qc to prevent replay.
                let mut wish_ok = false;
                for epoch in self.verification_epochs() {
                    let bytes = crate::pacemaker::wish_signing_bytes(
                        &self.state.chain_id_hash,
                        epoch,
                        *target_view,
                        highest_qc.as_ref(),
                    );
                    if self.verifier.verify(&vi.public_key, &bytes, signature) {
                        wish_ok = true;
                        break;
                    }
                }
                if !wish_ok {
                    warn!(validator = %validator, "invalid wish signature");
                    return false;
                }
                true
            }
            ConsensusMessage::TimeoutCert(tc) => {
                // The TC's aggregate signature is a collection of individual Ed25519 signatures,
                // each signed over wish_signing_bytes(target_view, signer_highest_qc).
                // Because each validator may have a different highest_qc, we verify per-signer
                // using tc.highest_qcs[i] (indexed by validator slot).
                // This also enforces quorum: we sum voting power of verified signers.
                let target_view = ViewNumber(tc.view.as_u64() + 1);
                let n = vs.validator_count();
                if tc.aggregate_signature.signers.len() != n {
                    warn!(view = %tc.view, "TC signers bitfield length mismatch");
                    return false;
                }
                let mut sig_idx = 0usize;
                let mut power = 0u64;
                for (i, &signed) in tc.aggregate_signature.signers.iter().enumerate() {
                    if !signed {
                        continue;
                    }
                    let Some(vi) = vs.validators().get(i) else {
                        warn!(view = %tc.view, validator_idx = i, "TC signer index out of validator set");
                        return false;
                    };
                    let hqc = tc.highest_qcs.get(i).and_then(|h| h.as_ref());
                    if sig_idx >= tc.aggregate_signature.signatures.len() {
                        warn!(view = %tc.view, "TC aggregate_signature has fewer sigs than signers");
                        return false;
                    }
                    let mut tc_sig_ok = false;
                    for epoch in self.verification_epochs() {
                        let bytes = crate::pacemaker::wish_signing_bytes(
                            &self.state.chain_id_hash,
                            epoch,
                            target_view,
                            hqc,
                        );
                        if self.verifier.verify(
                            &vi.public_key,
                            &bytes,
                            &tc.aggregate_signature.signatures[sig_idx],
                        ) {
                            tc_sig_ok = true;
                            break;
                        }
                    }
                    if !tc_sig_ok {
                        warn!(view = %tc.view, validator = %vi.id, "TC signer signature invalid");
                        return false;
                    }
                    power += vs.power_of(vi.id);
                    sig_idx += 1;
                }
                if sig_idx != tc.aggregate_signature.signatures.len() {
                    warn!(view = %tc.view, "TC has extra signatures beyond bitfield");
                    return false;
                }
                if power < vs.quorum_threshold() {
                    warn!(view = %tc.view, power, threshold = vs.quorum_threshold(), "TC insufficient quorum");
                    return false;
                }
                true
            }
            ConsensusMessage::StatusCert {
                locked_qc,
                validator,
                signature,
            } => {
                let Some(vi) = vs.get(*validator) else {
                    warn!(validator = %validator, "status from unknown validator");
                    return false;
                };
                let mut status_ok = false;
                for epoch in self.verification_epochs() {
                    let bytes = view_protocol::status_signing_bytes(
                        &self.state.chain_id_hash,
                        epoch,
                        self.state.current_view,
                        locked_qc,
                    );
                    if self.verifier.verify(&vi.public_key, &bytes, signature) {
                        status_ok = true;
                        break;
                    }
                }
                if !status_ok {
                    warn!(validator = %validator, "invalid status signature");
                    return false;
                }
                true
            }
            ConsensusMessage::Evidence(_) => {
                // Evidence gossip does not carry an outer signature;
                // the proof itself contains the conflicting vote signatures
                // which are verified by the application layer.
                true
            }
        }
    }

    async fn handle_message(
        &mut self,
        _sender: Option<ValidatorId>,
        msg: ConsensusMessage,
    ) -> Result<()> {
        // Run signature verification in a blocking context so that the tokio
        // event loop is not stalled by CPU-intensive Ed25519 batch operations.
        // block_in_place yields the current thread to the scheduler while the
        // blocking work runs, keeping timers and I/O tasks responsive.
        let verified = tokio::task::block_in_place(|| self.verify_message(&msg));
        if !verified {
            return Ok(());
        }

        match msg {
            ConsensusMessage::Propose {
                block,
                justify,
                double_cert,
                signature: _,
            } => {
                let block = *block;
                let justify = *justify;
                let double_cert = double_cert.map(|dc| *dc);

                // If proposal is from a future view, advance to it first
                if block.view > self.state.current_view {
                    if let Some(ref dc) = double_cert {
                        if !tokio::task::block_in_place(|| self.validate_double_cert(dc)) {
                            return Ok(());
                        }

                        // Fast-forward via double cert
                        self.apply_commit(dc, "fast-forward").await;
                        self.state.highest_double_cert = Some(dc.clone());
                        self.advance_view_to(block.view, ViewEntryTrigger::DoubleCert(dc.clone()))
                            .await;
                    } else {
                        return Ok(());
                    }
                } else if block.view < self.state.current_view {
                    // Still store blocks from past views if we haven't committed
                    // that height yet. This handles the case where fast-forward
                    // advanced our view but we missed storing the block from the
                    // earlier proposal. Without this, chain commits that walk
                    // the parent chain would fail with "block not found".
                    if block.height > self.state.last_committed_height {
                        // Verify block hash before storing past-view blocks
                        let expected = hotmint_crypto::compute_block_hash(&block);
                        if block.hash == expected {
                            let mut store = self.store.write();
                            store.put_block(block);
                        }
                    }
                    return Ok(());
                }

                let mut store = self.store.write();

                // R-25: verify any DoubleCert in the same-view proposal path.
                // The future-view path already calls validate_double_cert; the same-view path
                // passes the DC straight to on_proposal → try_commit without verification.
                // A Byzantine leader could inject a forged DC to trigger incorrect commits.
                if let Some(ref dc) = double_cert
                    && !tokio::task::block_in_place(|| self.validate_double_cert(dc))
                {
                    return Ok(());
                }

                // R-28: persist justify QC as commit evidence for the block it certifies.
                // When blocks are committed via the 2-chain rule (possibly multiple blocks at
                // once), the innermost block gets its own commit QC, but ancestor blocks only
                // get the chain-rule commit and have no stored QC.  Storing the justify QC here
                // ensures that sync responders can later serve those ancestor blocks with proof.
                if justify.aggregate_signature.count() > 0
                    && let Some(justified_block) = store.get_block(&justify.block_hash)
                    && store.get_commit_qc(justified_block.height).is_none()
                {
                    store.put_commit_qc(justified_block.height, justify.clone());
                }

                // WAL: log commit intent before fast-forward commit in on_proposal.
                if let Some(ref dc) = double_cert
                    && let Some(ref mut wal) = self.wal
                    && let Some(target_block) = store.get_block(&dc.inner_qc.block_hash)
                    && let Err(e) = wal.log_commit_intent(target_block.height)
                {
                    warn!(error = %e, "WAL: failed to log commit intent for fast-forward");
                }

                let proposal_result = view_protocol::on_proposal(
                    &mut self.state,
                    view_protocol::ProposalData {
                        block,
                        justify,
                        double_cert,
                    },
                    store.as_mut(),
                    self.network.as_ref(),
                    self.app.as_ref(),
                    self.signer.as_ref(),
                )
                .c(d!())?;
                drop(store);

                // Process fast-forward commit result (WAL, tx indexing,
                // evidence marking, liveness tracking, persist_state).
                if let Some(result) = proposal_result.commit_result {
                    self.process_commit_result(&result);
                }
                if let Some(epoch) = proposal_result.pending_epoch {
                    self.pending_epoch = Some(epoch);
                }
            }

            ConsensusMessage::VoteMsg(vote) => {
                if vote.view != self.state.current_view {
                    return Ok(());
                }
                if !self.state.is_leader() {
                    return Ok(());
                }
                if vote.vote_type != VoteType::Vote {
                    return Ok(());
                }

                let result = self
                    .vote_collector
                    .add_vote(
                        &self.state.validator_set,
                        vote,
                        self.state.current_epoch.number,
                    )
                    .c(d!())?;
                self.handle_equivocation(&result);
                if let Some(qc) = result.qc {
                    self.on_qc_formed(qc).await;
                }
            }

            ConsensusMessage::Prepare {
                certificate,
                signature: _,
            } => {
                if certificate.view < self.state.current_view {
                    return Ok(());
                }
                if certificate.view == self.state.current_view {
                    // Validate the Prepare's block app_hash if we have the block in
                    // store. Prevents locking onto a block whose app_hash diverges from
                    // our local state. When the block is absent (node caught up via TC),
                    // we defer to the QC's 2f+1 signatures for safety.
                    let store = self.store.read();
                    let block_opt = store.get_block(&certificate.block_hash);
                    if self.app.tracks_app_hash()
                        && let Some(ref block) = block_opt
                        && block.app_hash != self.state.last_app_hash
                    {
                        warn!(
                            block_app_hash = %block.app_hash,
                            local_app_hash = %self.state.last_app_hash,
                            "prepare block app_hash mismatch, ignoring"
                        );
                        return Ok(());
                    }

                    // Generate vote extension for Vote2 (ABCI++ Vote Extensions).
                    // Only if we have the block available and have voting power.
                    let vote_extension = block_opt.and_then(|block| {
                        let ctx = BlockContext {
                            height: block.height,
                            view: self.state.current_view,
                            proposer: block.proposer,
                            epoch: self.state.current_epoch.number,
                            epoch_start_view: self.state.current_epoch.start_view,
                            validator_set: &self.state.validator_set,
                            vote_extensions: vec![],
                        };
                        self.app.extend_vote(&block, &ctx)
                    });
                    drop(store);

                    view_protocol::on_prepare(
                        &mut self.state,
                        certificate,
                        self.network.as_ref(),
                        self.signer.as_ref(),
                        vote_extension,
                    );
                }
            }

            ConsensusMessage::Vote2Msg(vote) => {
                if vote.view != self.state.current_view {
                    return Ok(());
                }
                if vote.vote_type != VoteType::Vote2 {
                    return Ok(());
                }

                // Verify vote extension (ABCI++ Vote Extensions) if present.
                if let Some(ref ext) = vote.extension
                    && !self
                        .app
                        .verify_vote_extension(ext, &vote.block_hash, vote.validator)
                {
                    warn!(
                        validator = %vote.validator,
                        view = %vote.view,
                        "rejecting vote2: invalid vote extension"
                    );
                    return Ok(());
                }

                let result = self
                    .vote_collector
                    .add_vote(
                        &self.state.validator_set,
                        vote,
                        self.state.current_epoch.number,
                    )
                    .c(d!())?;
                self.handle_equivocation(&result);
                if let Some(outer_qc) = result.qc {
                    self.on_double_cert_formed(outer_qc, result.extensions)
                        .await;
                }
            }

            ConsensusMessage::Wish {
                target_view,
                validator,
                highest_qc,
                signature,
            } => {
                // Validate carried highest_qc (C4 mitigation).
                // Both signature authenticity and 2f+1 quorum weight must pass.
                if let Some(ref qc) = highest_qc
                    && qc.aggregate_signature.count() > 0
                {
                    let qc_bytes = Vote::signing_bytes(
                        &self.state.chain_id_hash,
                        qc.epoch,
                        qc.view,
                        &qc.block_hash,
                        VoteType::Vote,
                    );
                    if !tokio::task::block_in_place(|| {
                        self.verifier.verify_aggregate(
                            &self.state.validator_set,
                            &qc_bytes,
                            &qc.aggregate_signature,
                        )
                    }) {
                        warn!(validator = %validator, "wish carries invalid highest_qc signature");
                        return Ok(());
                    }
                    // Only enforce quorum against the current validator set if the
                    // QC was formed in the current epoch. A QC from a previous epoch
                    // may not meet the new set's quorum threshold (e.g., after a
                    // validator power change), but its signatures were already verified
                    // above, so it remains a valid proof of finality in its own epoch.
                    if qc.epoch == self.state.current_epoch.number
                        && !hotmint_crypto::has_quorum(
                            &self.state.validator_set,
                            &qc.aggregate_signature,
                        )
                    {
                        warn!(validator = %validator, "wish carries highest_qc without quorum");
                        return Ok(());
                    }
                }

                if let Some(tc) = self.pacemaker.add_wish(
                    &self.state.validator_set,
                    target_view,
                    validator,
                    highest_qc,
                    signature,
                ) {
                    info!(
                        validator = %self.state.validator_id,
                        view = %tc.view,
                        "TC formed, advancing view"
                    );
                    self.network
                        .broadcast(ConsensusMessage::TimeoutCert(tc.clone()));
                    self.advance_view(ViewEntryTrigger::TimeoutCert(tc)).await;
                }
            }

            ConsensusMessage::TimeoutCert(tc) => {
                if self.pacemaker.should_relay_tc(&tc) {
                    self.network
                        .broadcast(ConsensusMessage::TimeoutCert(tc.clone()));
                }
                let new_view = ViewNumber(tc.view.as_u64() + 1);
                if new_view > self.state.current_view {
                    self.advance_view(ViewEntryTrigger::TimeoutCert(tc)).await;
                }
            }

            ConsensusMessage::StatusCert {
                locked_qc,
                validator,
                signature: _,
            } => {
                if self.state.is_leader() && self.state.step == ViewStep::WaitingForStatus {
                    if let Some(ref qc) = locked_qc {
                        self.state.update_highest_qc(qc);
                    }
                    self.status_senders.insert(validator);
                    let status_power: u64 = self
                        .status_senders
                        .iter()
                        .map(|v| self.state.validator_set.power_of(*v))
                        .sum();
                    // Leader's own power counts toward quorum
                    let total_power =
                        status_power + self.state.validator_set.power_of(self.state.validator_id);
                    if total_power >= self.state.validator_set.quorum_threshold() {
                        self.try_propose().await;
                    }
                }
            }

            ConsensusMessage::Evidence(proof) => {
                // C-6: Cryptographically verify evidence before accepting.
                // Both signatures must be valid from the alleged validator for
                // different block hashes at the same (view, vote_type).
                let vs = &self.state.validator_set;
                let vi = match vs.get(proof.validator) {
                    Some(vi) => vi,
                    None => {
                        warn!(validator = %proof.validator, "evidence for unknown validator");
                        return Ok(());
                    }
                };
                if proof.block_hash_a == proof.block_hash_b {
                    warn!(validator = %proof.validator, "evidence has identical block hashes");
                    return Ok(());
                }
                // Use the epoch from the proof itself (not local epoch) so
                // cross-epoch evidence can be verified correctly.
                let bytes_a = Vote::signing_bytes(
                    &self.state.chain_id_hash,
                    proof.epoch,
                    proof.view,
                    &proof.block_hash_a,
                    proof.vote_type,
                );
                let bytes_b = Vote::signing_bytes(
                    &self.state.chain_id_hash,
                    proof.epoch,
                    proof.view,
                    &proof.block_hash_b,
                    proof.vote_type,
                );
                if !self
                    .verifier
                    .verify(&vi.public_key, &bytes_a, &proof.signature_a)
                    || !self
                        .verifier
                        .verify(&vi.public_key, &bytes_b, &proof.signature_b)
                {
                    warn!(validator = %proof.validator, "evidence has invalid signatures");
                    return Ok(());
                }

                info!(
                    validator = %proof.validator,
                    view = %proof.view,
                    "received valid evidence gossip"
                );
                if let Err(e) = self.app.on_evidence(&proof) {
                    warn!(error = %e, "on_evidence callback failed for gossiped proof");
                }
                if let Some(ref mut store) = self.evidence_store {
                    store.put_evidence(proof);
                }
            }
        }
        Ok(())
    }

    fn handle_equivocation(&mut self, result: &crate::vote_collector::VoteResult) {
        if let Some(ref proof) = result.equivocation {
            warn!(
                validator = %proof.validator,
                view = %proof.view,
                "equivocation detected!"
            );
            if let Err(e) = self.app.on_evidence(proof) {
                warn!(error = %e, "on_evidence callback failed");
            }
            self.network.broadcast_evidence(proof);
            if let Some(ref mut store) = self.evidence_store {
                store.put_evidence(proof.clone());
            }
        }
    }

    async fn on_qc_formed(&mut self, qc: QuorumCertificate) {
        // Save the QC so we can reliably pair it when forming a DoubleCert
        self.current_view_qc = Some(qc.clone());

        view_protocol::on_votes_collected(
            &mut self.state,
            qc.clone(),
            self.network.as_ref(),
            self.signer.as_ref(),
        );

        // Leader also does vote2 for its own prepare (self-vote for step 5)
        // Generate vote extension (ABCI++ Vote Extensions) if the block is available.
        let vote_extension = {
            let store = self.store.read();
            store.get_block(&qc.block_hash).and_then(|block| {
                let ctx = BlockContext {
                    height: block.height,
                    view: self.state.current_view,
                    proposer: block.proposer,
                    epoch: self.state.current_epoch.number,
                    epoch_start_view: self.state.current_epoch.start_view,
                    validator_set: &self.state.validator_set,
                    vote_extensions: vec![],
                };
                self.app.extend_vote(&block, &ctx)
            })
        };
        let vote_bytes = Vote::signing_bytes(
            &self.state.chain_id_hash,
            self.state.current_epoch.number,
            self.state.current_view,
            &qc.block_hash,
            VoteType::Vote2,
        );
        let signature = self.signer.sign(&vote_bytes);
        let vote = Vote {
            block_hash: qc.block_hash,
            view: self.state.current_view,
            validator: self.state.validator_id,
            signature,
            vote_type: VoteType::Vote2,
            extension: vote_extension,
        };

        // Lock on this QC
        self.state.update_locked_qc(&qc);

        let next_leader_id =
            leader::next_leader(&self.state.validator_set, self.state.current_view);
        if next_leader_id == self.state.validator_id {
            // We are the next leader, collect vote2 locally
            match self.vote_collector.add_vote(
                &self.state.validator_set,
                vote,
                self.state.current_epoch.number,
            ) {
                Ok(result) => {
                    self.handle_equivocation(&result);
                    if let Some(outer_qc) = result.qc {
                        self.on_double_cert_formed(outer_qc, result.extensions)
                            .await;
                    }
                }
                Err(e) => warn!(error = %e, "failed to add self vote2"),
            }
        } else {
            self.network
                .send_to(next_leader_id, ConsensusMessage::Vote2Msg(vote));
        }
    }

    async fn on_double_cert_formed(
        &mut self,
        outer_qc: QuorumCertificate,
        extensions: Vec<(ValidatorId, Vec<u8>)>,
    ) {
        // Use the QC we explicitly saved from this view's first voting round
        let inner_qc = match self.current_view_qc.take() {
            Some(qc) if qc.block_hash == outer_qc.block_hash => qc,
            _ => {
                // Fallback to locked_qc or highest_qc
                match &self.state.locked_qc {
                    Some(qc) if qc.block_hash == outer_qc.block_hash => qc.clone(),
                    _ => match &self.state.highest_qc {
                        Some(qc) if qc.block_hash == outer_qc.block_hash => qc.clone(),
                        _ => {
                            warn!(
                                validator = %self.state.validator_id,
                                "double cert formed but can't find matching inner QC"
                            );
                            return;
                        }
                    },
                }
            }
        };

        let dc = DoubleCertificate {
            inner_qc,
            outer_qc,
            vote_extensions: extensions,
        };

        info!(
            validator = %self.state.validator_id,
            view = %self.state.current_view,
            hash = %dc.inner_qc.block_hash,
            "double certificate formed, committing"
        );

        // Commit
        self.apply_commit(&dc, "double-cert").await;

        self.state.highest_double_cert = Some(dc.clone());

        // Advance to next view — as new leader, include DC in proposal
        self.advance_view(ViewEntryTrigger::DoubleCert(dc)).await;
    }

    async fn handle_timeout(&mut self) {
        // Skip wish building/signing entirely when we have no voting power (fullnodes).
        // build_wish involves a cryptographic signing operation that serves no purpose
        // when the wish will never be broadcast or counted toward a TC.
        let has_power = self.state.validator_set.power_of(self.state.validator_id) > 0;
        if !has_power {
            self.pacemaker.on_timeout();
            return;
        }

        info!(
            validator = %self.state.validator_id,
            view = %self.state.current_view,
            "view timeout, sending wish"
        );

        let wish = self.pacemaker.build_wish(
            &self.state.chain_id_hash,
            self.state.current_epoch.number,
            self.state.current_view,
            self.state.validator_id,
            self.state.highest_qc.clone(),
            self.signer.as_ref(),
        );

        self.network.broadcast(wish.clone());

        // Also process our own wish
        if let ConsensusMessage::Wish {
            target_view,
            validator,
            highest_qc,
            signature,
        } = wish
            && let Some(tc) = self.pacemaker.add_wish(
                &self.state.validator_set,
                target_view,
                validator,
                highest_qc,
                signature,
            )
        {
            self.network
                .broadcast(ConsensusMessage::TimeoutCert(tc.clone()));
            self.advance_view(ViewEntryTrigger::TimeoutCert(tc)).await;
            return;
        }

        // Exponential backoff on repeated timeouts
        self.pacemaker.on_timeout();
    }

    /// Post-commit processing: store commit QCs, index txs, mark evidence,
    /// track liveness, persist state, and log WAL done.
    /// Shared by both `apply_commit` (normal DC path) and the on_proposal
    /// fast-forward path to ensure all commit side-effects happen consistently.
    fn process_commit_result(&mut self, result: &CommitResult) {
        if result.committed_blocks.is_empty() {
            return;
        }
        {
            let mut s = self.store.write();
            for (i, block) in result.committed_blocks.iter().enumerate() {
                if result.commit_qc.block_hash == block.hash {
                    s.put_commit_qc(block.height, result.commit_qc.clone());
                }
                let txs = crate::commit::decode_payload(&block.payload);
                for (tx_idx, tx) in txs.iter().enumerate() {
                    let tx_hash = *blake3::hash(tx).as_bytes();
                    s.put_tx_index(tx_hash, block.height, tx_idx as u32);
                }
                if let Some(resp) = result.block_responses.get(i) {
                    s.put_block_results(block.height, resp.clone());
                }
            }
            s.flush();
        }
        if let Some(ref mut ev_store) = self.evidence_store {
            for block in &result.committed_blocks {
                for proof in &block.evidence {
                    ev_store.mark_committed(proof.view, proof.validator);
                }
                for proof in ev_store.get_pending() {
                    if proof.view <= block.view {
                        ev_store.mark_committed(proof.view, proof.validator);
                    }
                }
            }
        }
        self.liveness_tracker.record_commit(
            &self.state.validator_set,
            &result.commit_qc.aggregate_signature.signers,
        );
        self.persist_state();
        if let Some(ref mut wal) = self.wal
            && let Err(e) = wal.log_commit_done(self.state.last_committed_height)
        {
            warn!(error = %e, "WAL: failed to log commit done");
        }
    }

    /// Apply the result of a successful try_commit: update app_hash, pending epoch,
    /// store commit QCs, and flush. Called from both normal and fast-forward commit paths.
    async fn apply_commit(&mut self, dc: &DoubleCertificate, context: &str) {
        // WAL: log commit intent before executing blocks.
        if let Some(ref mut wal) = self.wal {
            let target_height = {
                let store = self.store.read();
                store.get_block(&dc.inner_qc.block_hash).map(|b| b.height)
            };
            if let Some(h) = target_height
                && let Err(e) = wal.log_commit_intent(h)
            {
                warn!(error = %e, "WAL: failed to log commit intent");
            }
        }

        let store = self.store.read();
        match try_commit(
            dc,
            store.as_ref(),
            self.app.as_ref(),
            &mut self.state.last_committed_height,
            &self.state.current_epoch,
        ) {
            Ok(result) => {
                if !result.committed_blocks.is_empty() {
                    self.state.last_app_hash = result.last_app_hash;
                }
                if result.pending_epoch.is_some() {
                    self.pending_epoch = result.pending_epoch.clone();
                }
                drop(store);
                self.process_commit_result(&result);
            }
            Err(e) => {
                warn!(error = %e, "try_commit failed during {context}");
                drop(store);
            }
        }
    }

    /// Cryptographically validate a DoubleCertificate:
    /// 1. inner and outer QC must reference the same block hash
    /// 2. inner QC aggregate signature (Vote1) must be valid and reach quorum
    /// 3. outer QC aggregate signature (Vote2) must be valid and reach quorum
    ///
    /// Note on quorum and epoch transitions: DCs are always formed in the same epoch as
    /// the block they commit (vote_collector enforces quorum at formation time).  When
    /// a DC is received by a node that has already transitioned to a new epoch, the
    /// validator set may differ.  We enforce quorum against the current validator set
    /// as the best available reference; a legitimate DC from a prior epoch should still
    /// satisfy quorum against the new set unless the set shrank significantly.
    fn validate_double_cert(&self, dc: &DoubleCertificate) -> bool {
        if dc.inner_qc.block_hash != dc.outer_qc.block_hash {
            warn!("double cert inner/outer block_hash mismatch");
            return false;
        }
        let vs = &self.state.validator_set;
        let inner_bytes = Vote::signing_bytes(
            &self.state.chain_id_hash,
            dc.inner_qc.epoch,
            dc.inner_qc.view,
            &dc.inner_qc.block_hash,
            VoteType::Vote,
        );
        if !self
            .verifier
            .verify_aggregate(vs, &inner_bytes, &dc.inner_qc.aggregate_signature)
        {
            warn!("double cert inner QC signature invalid");
            return false;
        }
        if !hotmint_crypto::has_quorum(vs, &dc.inner_qc.aggregate_signature) {
            warn!("double cert inner QC below quorum threshold");
            return false;
        }
        let outer_bytes = Vote::signing_bytes(
            &self.state.chain_id_hash,
            dc.outer_qc.epoch,
            dc.outer_qc.view,
            &dc.outer_qc.block_hash,
            VoteType::Vote2,
        );
        if !self
            .verifier
            .verify_aggregate(vs, &outer_bytes, &dc.outer_qc.aggregate_signature)
        {
            warn!("double cert outer QC signature invalid");
            return false;
        }
        if !hotmint_crypto::has_quorum(vs, &dc.outer_qc.aggregate_signature) {
            warn!("double cert outer QC below quorum threshold");
            return false;
        }
        true
    }

    fn persist_state(&mut self) {
        if let Some(p) = self.persistence.as_mut() {
            p.save_current_view(self.state.current_view);
            if let Some(ref qc) = self.state.locked_qc {
                p.save_locked_qc(qc);
            }
            if let Some(ref qc) = self.state.highest_qc {
                p.save_highest_qc(qc);
            }
            p.save_last_committed_height(self.state.last_committed_height);
            p.save_current_epoch(&self.state.current_epoch);
            p.save_last_app_hash(self.state.last_app_hash);
            p.flush();
        }
    }

    async fn advance_view(&mut self, trigger: ViewEntryTrigger) {
        let new_view = match &trigger {
            ViewEntryTrigger::DoubleCert(_) => self.state.current_view.next(),
            ViewEntryTrigger::TimeoutCert(tc) => ViewNumber(tc.view.as_u64() + 1),
            ViewEntryTrigger::Genesis => ViewNumber(1),
        };
        self.advance_view_to(new_view, trigger).await;
    }

    async fn advance_view_to(&mut self, new_view: ViewNumber, trigger: ViewEntryTrigger) {
        if new_view <= self.state.current_view {
            return;
        }

        // Reset backoff on successful progress (DoubleCert path)
        let is_progress = matches!(&trigger, ViewEntryTrigger::DoubleCert(_));

        // Capture vote extensions from DoubleCertificate for the next create_payload.
        if let ViewEntryTrigger::DoubleCert(ref dc) = trigger {
            self.state.pending_vote_extensions = dc.vote_extensions.clone();
        } else {
            self.state.pending_vote_extensions.clear();
        }

        self.vote_collector.clear_view(self.state.current_view);
        self.vote_collector.prune_before(self.state.current_view);
        self.pacemaker.clear_view(self.state.current_view);
        self.pacemaker.prune_before(self.state.current_view);
        self.status_senders.clear();
        self.current_view_qc = None;

        // Epoch transition: apply pending validator set change when we reach the
        // epoch's start_view. The start_view is set deterministically (commit_view + 2)
        // so all honest nodes apply the transition at the same view.
        if self
            .pending_epoch
            .as_ref()
            .is_some_and(|e| new_view >= e.start_view)
        {
            // SAFETY: we just verified `pending_epoch` is `Some` above.
            let Some(new_epoch) = self.pending_epoch.take() else {
                unreachable!("pending_epoch was Some in the condition check");
            };
            info!(
                validator = %self.state.validator_id,
                old_epoch = %self.state.current_epoch.number,
                new_epoch = %new_epoch.number,
                start_view = %new_epoch.start_view,
                validators = new_epoch.validator_set.validator_count(),
                "epoch transition"
            );
            // Report offline validators to the application before transitioning.
            let offline = self.liveness_tracker.offline_validators();
            if !offline.is_empty() {
                let evidence: Vec<OfflineEvidence> = offline
                    .iter()
                    .map(|&(validator, missed, total)| OfflineEvidence {
                        validator,
                        missed_commits: missed,
                        total_commits: total,
                        evidence_height: self.state.last_committed_height,
                    })
                    .collect();
                info!(
                    offline_count = evidence.len(),
                    epoch = %self.state.current_epoch.number,
                    "reporting offline validators"
                );
                if let Err(e) = self.app.on_offline_validators(&evidence) {
                    warn!(error = %e, "on_offline_validators callback failed");
                }
            }
            self.liveness_tracker.reset();

            self.state.validator_set = new_epoch.validator_set.clone();
            self.state.current_epoch = new_epoch;
            // Notify network layer of the new validator set and epoch
            self.network
                .on_epoch_change(self.state.current_epoch.number, &self.state.validator_set);
            // Full clear: old votes/wishes are from the previous epoch's validator set
            self.vote_collector = VoteCollector::new();
            self.pacemaker = Pacemaker::with_config(self.pacemaker_config.clone());
        }

        view_protocol::enter_view(
            &mut self.state,
            new_view,
            trigger,
            self.network.as_ref(),
            self.signer.as_ref(),
        );

        if is_progress {
            self.pacemaker.reset_on_progress();
        } else {
            self.pacemaker.reset_timer();
        }

        self.persist_state();

        // If we're the leader, propose immediately.
        // Note: in a full implementation, the leader would collect StatusCerts
        // before proposing (status_senders quorum gate). Currently the immediate
        // propose path is required for liveness across epoch transitions where
        // cross-epoch verification complexity can stall status collection.
        if self.state.is_leader() && self.state.step == ViewStep::WaitingForStatus {
            self.try_propose().await;
        }
    }
}

// ---------------------------------------------------------------------------
// Regression tests for sub-quorum certificate injection (R-29, R-32)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use parking_lot::RwLock;

    use hotmint_crypto::{Ed25519Signer, Ed25519Verifier};
    use hotmint_types::Signer as SignerTrait;
    use hotmint_types::certificate::QuorumCertificate;
    use hotmint_types::crypto::AggregateSignature;
    use hotmint_types::epoch::EpochNumber;
    use hotmint_types::validator::{ValidatorId, ValidatorInfo};
    use hotmint_types::vote::{Vote, VoteType};
    use tokio::sync::mpsc;

    use crate::application::NoopApplication;
    use crate::network::NetworkSink;
    use crate::state::ConsensusState;
    use crate::store::MemoryBlockStore;

    // Minimal no-op network for unit tests — messages are silently discarded.
    struct DevNullNetwork;
    impl NetworkSink for DevNullNetwork {
        fn broadcast(&self, _: ConsensusMessage) {}
        fn send_to(&self, _: ValidatorId, _: ConsensusMessage) {}
    }

    /// Default chain_id_hash for tests — matches ConsensusState::new() which uses chain_id = "".
    fn test_chain_id_hash() -> [u8; 32] {
        *blake3::hash(b"").as_bytes()
    }

    fn make_validator_set_4() -> (ValidatorSet, Vec<Ed25519Signer>) {
        let signers: Vec<Ed25519Signer> = (0..4)
            .map(|i| Ed25519Signer::generate(ValidatorId(i)))
            .collect();
        let infos: Vec<ValidatorInfo> = signers
            .iter()
            .map(|s| ValidatorInfo {
                id: s.validator_id(),
                public_key: s.public_key(),
                power: 1,
            })
            .collect();
        (ValidatorSet::new(infos), signers)
    }

    fn make_test_engine(
        vid: ValidatorId,
        vs: ValidatorSet,
        signer: Ed25519Signer,
    ) -> (
        ConsensusEngine,
        mpsc::Sender<(Option<ValidatorId>, ConsensusMessage)>,
    ) {
        let (tx, rx) = mpsc::channel(64);
        let store = Arc::new(RwLock::new(
            Box::new(MemoryBlockStore::new()) as Box<dyn crate::store::BlockStore>
        ));
        let state = ConsensusState::new(vid, vs);
        let engine = ConsensusEngine::new(
            state,
            store,
            Box::new(DevNullNetwork),
            Box::new(NoopApplication),
            Box::new(signer),
            rx,
            EngineConfig {
                verifier: Box::new(Ed25519Verifier),
                pacemaker: None,
                persistence: None,
                evidence_store: None,
                wal: None,
            },
        );
        (engine, tx)
    }

    // R-29 regression: a Propose message whose justify QC is signed by fewer than
    // 2f+1 validators must be rejected by verify_message().
    #[test]
    fn r29_propose_sub_quorum_justify_rejected_by_verify_message() {
        let (vs, signers) = make_validator_set_4();
        // Use a fresh signer for the engine; verify_message only needs the engine's
        // validator set and verifier, not its own signing key.
        let engine_signer = Ed25519Signer::generate(ValidatorId(0));
        let (engine, _tx) = make_test_engine(ValidatorId(0), vs.clone(), engine_signer);

        // Build a justify QC signed by exactly 1 of 4 validators — below 2f+1 = 3.
        let chain_id_hash = test_chain_id_hash();
        let hash = BlockHash::GENESIS;
        let qc_view = ViewNumber::GENESIS;
        let vote_bytes = Vote::signing_bytes(
            &chain_id_hash,
            EpochNumber(0),
            qc_view,
            &hash,
            VoteType::Vote,
        );
        let mut agg = AggregateSignature::new(4);
        agg.add(1, SignerTrait::sign(&signers[1], &vote_bytes))
            .unwrap();
        let sub_quorum_qc = QuorumCertificate {
            block_hash: hash,
            view: qc_view,
            aggregate_signature: agg,
            epoch: EpochNumber(0),
        };

        // Construct a proposal from V1 carrying this sub-quorum justify.
        let mut block = Block::genesis();
        block.height = Height(1);
        block.view = ViewNumber(1);
        block.proposer = ValidatorId(1);
        block.hash = block.compute_hash();
        let proposal_bytes = crate::view_protocol::proposal_signing_bytes(
            &chain_id_hash,
            EpochNumber(0),
            &block,
            &sub_quorum_qc,
        );
        let signature = SignerTrait::sign(&signers[1], &proposal_bytes);

        let msg = ConsensusMessage::Propose {
            block: Box::new(block),
            justify: Box::new(sub_quorum_qc),
            double_cert: None,
            signature,
        };

        assert!(
            !engine.verify_message(&msg),
            "R-29 regression: Propose with sub-quorum justify QC must be rejected by verify_message"
        );
    }

    // R-29 regression: a Propose message with a full quorum justify QC (3/4) must pass.
    #[test]
    fn r29_propose_full_quorum_justify_accepted_by_verify_message() {
        let (vs, signers) = make_validator_set_4();
        let engine_signer = Ed25519Signer::generate(ValidatorId(0));
        let (engine, _tx) = make_test_engine(ValidatorId(0), vs.clone(), engine_signer);

        let chain_id_hash = test_chain_id_hash();
        let hash = BlockHash::GENESIS;
        let qc_view = ViewNumber::GENESIS;
        let vote_bytes = Vote::signing_bytes(
            &chain_id_hash,
            EpochNumber(0),
            qc_view,
            &hash,
            VoteType::Vote,
        );
        // 3 of 4 signers — meets 2f+1 threshold.
        let mut agg = AggregateSignature::new(4);
        for (i, signer) in signers.iter().take(3).enumerate() {
            agg.add(i, SignerTrait::sign(signer, &vote_bytes)).unwrap();
        }
        let full_quorum_qc = QuorumCertificate {
            block_hash: hash,
            view: qc_view,
            aggregate_signature: agg,
            epoch: EpochNumber(0),
        };

        let mut block = Block::genesis();
        block.height = Height(1);
        block.view = ViewNumber(1);
        block.proposer = ValidatorId(1);
        block.hash = block.compute_hash();
        let proposal_bytes = crate::view_protocol::proposal_signing_bytes(
            &chain_id_hash,
            EpochNumber(0),
            &block,
            &full_quorum_qc,
        );
        let signature = SignerTrait::sign(&signers[1], &proposal_bytes);

        let msg = ConsensusMessage::Propose {
            block: Box::new(block),
            justify: Box::new(full_quorum_qc),
            double_cert: None,
            signature,
        };

        assert!(
            engine.verify_message(&msg),
            "R-29: Propose with full quorum justify QC must pass verify_message"
        );
    }

    // R-32 regression: a Wish carrying a sub-quorum highest_qc must cause
    // verify_highest_qc_in_wish to treat the QC as invalid and return false,
    // which causes handle_message to discard the Wish without forwarding it
    // to the pacemaker.
    //
    // We verify the sub-component: has_quorum returns false for a 1-of-4 aggregate,
    // ensuring the guard in handle_message fires.
    #[test]
    fn r32_sub_quorum_highest_qc_fails_has_quorum() {
        let (vs, signers) = make_validator_set_4();

        let chain_id_hash = test_chain_id_hash();
        let hash = BlockHash([1u8; 32]);
        let qc_view = ViewNumber(1);
        let vote_bytes = Vote::signing_bytes(
            &chain_id_hash,
            EpochNumber(0),
            qc_view,
            &hash,
            VoteType::Vote,
        );

        // Build a QC with only 1 signer — sub-quorum.
        let mut agg = AggregateSignature::new(4);
        agg.add(0, SignerTrait::sign(&signers[0], &vote_bytes))
            .unwrap();

        assert!(
            !hotmint_crypto::has_quorum(&vs, &agg),
            "R-32 regression: 1-of-4 signed QC must not satisfy has_quorum"
        );

        // Build a QC with 3 signers — full quorum.
        let mut agg_full = AggregateSignature::new(4);
        for (i, signer) in signers.iter().take(3).enumerate() {
            agg_full
                .add(i, SignerTrait::sign(signer, &vote_bytes))
                .unwrap();
        }
        assert!(
            hotmint_crypto::has_quorum(&vs, &agg_full),
            "R-32: 3-of-4 signed QC must satisfy has_quorum"
        );
    }
}
