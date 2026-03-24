use ruc::*;

use hotmint_types::Block;
use hotmint_types::context::{BlockContext, TxContext};
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::validator_update::EndBlockResponse;

/// Result of transaction validation, including priority for mempool ordering.
#[derive(Debug, Clone)]
pub struct TxValidationResult {
    /// Whether the transaction is valid.
    pub valid: bool,
    /// Priority for mempool ordering (higher = included first).
    /// Applications typically derive this from gas price / fee.
    pub priority: u64,
}

impl TxValidationResult {
    pub fn accept(priority: u64) -> Self {
        Self {
            valid: true,
            priority,
        }
    }

    pub fn reject() -> Self {
        Self {
            valid: false,
            priority: 0,
        }
    }
}

/// Application interface for the consensus engine.
///
/// The lifecycle for each committed block:
/// 1. `execute_block` — receives all decoded transactions at once; returns
///    validator updates and events
/// 2. `on_commit` — notification after the block is finalized
///
/// For block proposal:
/// - `create_payload` — build the payload bytes for a new block
///
/// For validation (before voting):
/// - `validate_block` — full block validation
/// - `validate_tx` — individual transaction validation for mempool
///
/// For evidence:
/// - `on_evidence` — called when equivocation is detected
///
/// All methods have default no-op implementations.
pub trait Application: Send + Sync {
    /// Create a payload for a new block proposal.
    /// Typically pulls transactions from the mempool.
    ///
    /// If your mempool is async, use `tokio::runtime::Handle::current().block_on(..)`
    /// to bridge into this synchronous callback.
    fn create_payload(&self, _ctx: &BlockContext) -> Vec<u8> {
        vec![]
    }

    /// Validate a proposed block before voting.
    fn validate_block(&self, _block: &Block, _ctx: &BlockContext) -> bool {
        true
    }

    /// Validate a single transaction for mempool admission.
    ///
    /// Returns a [`TxValidationResult`] with `valid` and `priority`.
    /// Priority determines ordering in the mempool (higher = included first).
    ///
    /// An optional [`TxContext`] provides the current chain height and epoch,
    /// which can be useful for state-dependent validation (nonce checks, etc.).
    fn validate_tx(&self, _tx: &[u8], _ctx: Option<&TxContext>) -> TxValidationResult {
        TxValidationResult::accept(0)
    }

    /// Execute an entire block in one call.
    ///
    /// Receives all decoded transactions from the block payload at once,
    /// allowing batch-optimised processing (bulk DB writes, parallel
    /// signature verification, etc.).
    ///
    /// Return [`EndBlockResponse`] with `validator_updates` to schedule an
    /// epoch transition, and/or `events` to emit application-defined events.
    fn execute_block(&self, _txs: &[&[u8]], _ctx: &BlockContext) -> Result<EndBlockResponse> {
        Ok(EndBlockResponse::default())
    }

    /// Called when a block is committed to the chain (notification).
    fn on_commit(&self, _block: &Block, _ctx: &BlockContext) -> Result<()> {
        Ok(())
    }

    /// Called when equivocation (double-voting) is detected.
    /// The application can use this to implement slashing.
    fn on_evidence(&self, _proof: &EquivocationProof) -> Result<()> {
        Ok(())
    }

    /// Query application state (returns opaque bytes).
    fn query(&self, _path: &str, _data: &[u8]) -> Result<Vec<u8>> {
        Ok(vec![])
    }

    /// List available state snapshots for state sync.
    fn list_snapshots(&self) -> Vec<hotmint_types::sync::SnapshotInfo> {
        vec![]
    }

    /// Load a chunk of a snapshot at the given height.
    fn load_snapshot_chunk(&self, _height: hotmint_types::Height, _chunk_index: u32) -> Vec<u8> {
        vec![]
    }

    /// Offer a snapshot to the application for state sync.
    fn offer_snapshot(
        &self,
        _snapshot: &hotmint_types::sync::SnapshotInfo,
    ) -> hotmint_types::sync::SnapshotOfferResult {
        hotmint_types::sync::SnapshotOfferResult::Reject
    }

    /// Apply a snapshot chunk received during state sync.
    fn apply_snapshot_chunk(
        &self,
        _chunk: Vec<u8>,
        _chunk_index: u32,
    ) -> hotmint_types::sync::ChunkApplyResult {
        hotmint_types::sync::ChunkApplyResult::Abort
    }

    /// Whether this application produces and verifies `app_hash` state roots.
    ///
    /// Applications that do not maintain a deterministic state root (e.g. the
    /// embedded [`NoopApplication`] used by fullnodes without an ABCI backend)
    /// should return `false`.  Sync will then bypass the app_hash equality
    /// check and accept the chain's authoritative value, allowing the node to
    /// follow a chain produced by peers running a real application.
    fn tracks_app_hash(&self) -> bool {
        true
    }
}

/// No-op application stub for testing and fullnode-without-ABCI mode.
pub struct NoopApplication;

impl Application for NoopApplication {
    /// NoopApplication does not maintain state, so app_hash tracking is skipped.
    fn tracks_app_hash(&self) -> bool {
        false
    }
}
