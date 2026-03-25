use serde::{Deserialize, Serialize};

use crate::block::Height;
use crate::epoch::EpochNumber;
use crate::validator::{ValidatorId, ValidatorSet};
use crate::view::ViewNumber;

/// Context provided to Application trait methods during block processing.
pub struct BlockContext<'a> {
    pub height: Height,
    pub view: ViewNumber,
    pub proposer: ValidatorId,
    pub epoch: EpochNumber,
    pub epoch_start_view: ViewNumber,
    pub validator_set: &'a ValidatorSet,
    /// Aggregated vote extensions from the previous round's Vote2 messages.
    /// Only populated for `create_payload` when the previous round committed
    /// via a DoubleCertificate whose Vote2 round carried extensions.
    pub vote_extensions: Vec<(ValidatorId, Vec<u8>)>,
}

/// Lightweight context for transaction validation (mempool admission).
/// Unlike [`BlockContext`], this does not require a specific block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxContext {
    pub height: Height,
    pub epoch: EpochNumber,
}

/// Owned version of [`BlockContext`] for cross-process IPC.
///
/// `BlockContext<'a>` borrows the `ValidatorSet`, which cannot be sent across
/// process boundaries. This type owns all its data and is serializable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedBlockContext {
    pub height: Height,
    pub view: ViewNumber,
    pub proposer: ValidatorId,
    pub epoch: EpochNumber,
    pub epoch_start_view: ViewNumber,
    pub validator_set: ValidatorSet,
    #[serde(default)]
    pub vote_extensions: Vec<(ValidatorId, Vec<u8>)>,
}

impl From<&BlockContext<'_>> for OwnedBlockContext {
    fn from(ctx: &BlockContext<'_>) -> Self {
        Self {
            height: ctx.height,
            view: ctx.view,
            proposer: ctx.proposer,
            epoch: ctx.epoch,
            epoch_start_view: ctx.epoch_start_view,
            validator_set: ctx.validator_set.clone(),
            vote_extensions: ctx.vote_extensions.clone(),
        }
    }
}
