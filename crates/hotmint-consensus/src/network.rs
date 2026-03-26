use hotmint_types::epoch::EpochNumber;
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::{ConsensusMessage, ValidatorId, ValidatorSet};

/// Message type for the consensus channel: `(sender_id, message)`.
pub type MsgSender = tokio::sync::mpsc::Sender<(Option<ValidatorId>, ConsensusMessage)>;
pub type MsgReceiver = tokio::sync::mpsc::Receiver<(Option<ValidatorId>, ConsensusMessage)>;

pub trait NetworkSink: Send + Sync {
    fn broadcast(&self, msg: ConsensusMessage);
    fn send_to(&self, target: ValidatorId, msg: ConsensusMessage);
    /// Notify the network layer of a validator set change (epoch transition).
    /// Default is no-op for test stubs.
    fn on_epoch_change(&self, _epoch: EpochNumber, _new_validator_set: &ValidatorSet) {}

    /// Broadcast equivocation evidence to all peers.
    /// Default is no-op for test stubs.
    fn broadcast_evidence(&self, _proof: &EquivocationProof) {}

    /// Broadcast a raw transaction to all connected peers via the mempool gossip protocol.
    /// Default is no-op for test stubs.
    fn broadcast_tx(&self, _tx_bytes: Vec<u8>) {}
}
