use hotmint_types::evidence::EquivocationProof;
use hotmint_types::validator::ValidatorId;
use hotmint_types::view::ViewNumber;

/// Storage backend for equivocation evidence.
///
/// The trait lives in `hotmint-consensus` so the engine can reference it
/// without a reverse dependency on `hotmint-storage`.
pub trait EvidenceStore: Send + Sync {
    /// Persist a new equivocation proof.
    fn put_evidence(&mut self, proof: EquivocationProof);

    /// Return all evidence that has not yet been marked as committed.
    fn get_pending(&self) -> Vec<EquivocationProof>;

    /// Mark evidence for the given `(view, validator)` as committed
    /// (i.e. included in a finalized block).
    fn mark_committed(&mut self, view: ViewNumber, validator: ValidatorId);

    /// Return every stored proof (committed or not).
    fn all(&self) -> Vec<EquivocationProof>;

    /// Flush pending writes to durable storage.
    fn flush(&self);
}
