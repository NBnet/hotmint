use serde::{Deserialize, Serialize};

use crate::block::Block;
use crate::certificate::{DoubleCertificate, QuorumCertificate, TimeoutCertificate};
use crate::crypto::Signature;
use crate::evidence::EquivocationProof;
use crate::validator::ValidatorId;
use crate::view::ViewNumber;
use crate::vote::Vote;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConsensusMessage {
    /// Leader broadcasts proposal
    Propose {
        block: Box<Block>,
        justify: Box<QuorumCertificate>,
        double_cert: Option<Box<DoubleCertificate>>,
        signature: Signature,
        /// Ancestor blocks needed for fast-forward commit via the double cert.
        /// When a proposal carries a DC, replicas who missed the DC's target
        /// block(s) need them to walk the commit chain. The leader includes
        /// all uncommitted ancestors referenced by the DC.
        #[serde(default)]
        ancestor_blocks: Vec<Block>,
    },

    /// First-phase vote → current leader
    VoteMsg(Vote),

    /// Leader broadcasts QC after collecting 2f+1 votes
    Prepare {
        certificate: QuorumCertificate,
        signature: Signature,
    },

    /// Second-phase vote → next leader
    Vote2Msg(Vote),

    /// Timeout wish: validator wants to advance to target_view
    Wish {
        target_view: ViewNumber,
        validator: ValidatorId,
        highest_qc: Option<QuorumCertificate>,
        signature: Signature,
    },

    /// Timeout certificate broadcast
    TimeoutCert(TimeoutCertificate),

    /// Status message: replica sends locked_qc to new leader
    StatusCert {
        locked_qc: Option<QuorumCertificate>,
        validator: ValidatorId,
        signature: Signature,
    },

    /// Evidence of validator equivocation (gossip)
    Evidence(EquivocationProof),
}
