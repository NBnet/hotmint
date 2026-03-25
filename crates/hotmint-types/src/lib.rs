pub mod block;
pub mod certificate;
pub mod context;
pub mod crypto;
pub mod epoch;
pub mod evidence;
pub mod message;
pub mod sync;
pub mod validator;
pub mod validator_update;
pub mod view;
pub mod vote;

pub use block::{Block, BlockHash, Height};
pub use certificate::{DoubleCertificate, QuorumCertificate, TimeoutCertificate};
pub use context::{BlockContext, OwnedBlockContext, TxContext};
pub use crypto::{AggregateSignature, PublicKey, Signature, Signer, Verifier};
pub use epoch::{Epoch, EpochNumber};
pub use evidence::EquivocationProof;
pub use message::ConsensusMessage;
pub use sync::{ChunkApplyResult, SnapshotInfo, SnapshotOfferResult, SyncRequest, SyncResponse};
pub use validator::{ValidatorId, ValidatorInfo, ValidatorSet};
pub use validator_update::{EndBlockResponse, Event, EventAttribute, ValidatorUpdate};
pub use view::ViewNumber;
pub use vote::{Vote, VoteType};

/// Response from an application query, optionally containing a Merkle proof.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct QueryResponse {
    /// The result data (opaque application bytes).
    pub data: Vec<u8>,
    /// Optional Merkle proof (application-defined format).
    /// Applications using IAVL/SMT trees can populate this to enable
    /// trustless verification by light clients.
    #[serde(default)]
    pub proof: Option<Vec<u8>>,
    /// Height at which the query was evaluated.
    #[serde(default)]
    pub height: u64,
}
