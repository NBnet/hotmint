use hotmint_types::block::{Block, BlockHash, Height};
use hotmint_types::context::{OwnedBlockContext, TxContext};
use hotmint_types::crypto::{PublicKey, Signature};
use hotmint_types::epoch::EpochNumber;
use hotmint_types::evidence::EquivocationProof;
use hotmint_types::sync::SnapshotInfo;
use hotmint_types::validator::{ValidatorId, ValidatorInfo, ValidatorSet};
use hotmint_types::validator_update::{EndBlockResponse, Event, EventAttribute, ValidatorUpdate};
use hotmint_types::view::ViewNumber;
use hotmint_types::vote::VoteType;

use crate::pb;

// ---- Block ----

impl From<&Block> for pb::Block {
    fn from(b: &Block) -> Self {
        Self {
            height: b.height.0,
            parent_hash: b.parent_hash.0.to_vec(),
            view: b.view.0,
            proposer: b.proposer.0,
            timestamp: b.timestamp,
            payload: b.payload.clone(),
            hash: b.hash.0.to_vec(),
            app_hash: b.app_hash.0.to_vec(),
            evidence: b.evidence.iter().map(|e| e.into()).collect(),
        }
    }
}

impl From<Block> for pb::Block {
    fn from(b: Block) -> Self {
        Self {
            height: b.height.0,
            parent_hash: b.parent_hash.0.to_vec(),
            view: b.view.0,
            proposer: b.proposer.0,
            timestamp: b.timestamp,
            payload: b.payload,
            hash: b.hash.0.to_vec(),
            app_hash: b.app_hash.0.to_vec(),
            evidence: b.evidence.into_iter().map(|e| e.into()).collect(),
        }
    }
}

impl TryFrom<pb::Block> for Block {
    type Error = prost::DecodeError;
    fn try_from(b: pb::Block) -> Result<Self, Self::Error> {
        Ok(Self {
            height: Height(b.height),
            parent_hash: bytes_to_hash(&b.parent_hash)?,
            view: ViewNumber(b.view),
            proposer: ValidatorId(b.proposer),
            timestamp: b.timestamp,
            payload: b.payload,
            app_hash: bytes_to_hash(&b.app_hash)?,
            evidence: b
                .evidence
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            hash: bytes_to_hash(&b.hash)?,
        })
    }
}

// ---- TxContext ----

impl From<&TxContext> for pb::TxContext {
    fn from(c: &TxContext) -> Self {
        Self {
            height: c.height.0,
            epoch: c.epoch.0,
        }
    }
}

impl From<pb::TxContext> for TxContext {
    fn from(c: pb::TxContext) -> Self {
        Self {
            height: Height(c.height),
            epoch: EpochNumber(c.epoch),
        }
    }
}

// ---- ValidatorInfo ----

impl From<&ValidatorInfo> for pb::ValidatorInfo {
    fn from(v: &ValidatorInfo) -> Self {
        Self {
            id: v.id.0,
            public_key: v.public_key.0.clone(),
            power: v.power,
        }
    }
}

impl TryFrom<pb::ValidatorInfo> for ValidatorInfo {
    type Error = prost::DecodeError;

    fn try_from(v: pb::ValidatorInfo) -> Result<Self, Self::Error> {
        Ok(Self {
            id: ValidatorId(v.id),
            public_key: bytes_to_public_key(&v.public_key)?,
            power: v.power,
        })
    }
}

// ---- ValidatorSet ----

impl From<&ValidatorSet> for pb::ValidatorSet {
    fn from(vs: &ValidatorSet) -> Self {
        Self {
            validators: vs
                .validators()
                .iter()
                .map(pb::ValidatorInfo::from)
                .collect(),
            total_power: vs.total_power(),
        }
    }
}

impl TryFrom<pb::ValidatorSet> for ValidatorSet {
    type Error = prost::DecodeError;

    fn try_from(vs: pb::ValidatorSet) -> Result<Self, Self::Error> {
        let infos: Vec<ValidatorInfo> = vs
            .validators
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;
        ValidatorSet::try_new(infos).map_err(prost::DecodeError::new)
    }
}

// ---- OwnedBlockContext ----

impl From<&OwnedBlockContext> for pb::BlockContext {
    fn from(c: &OwnedBlockContext) -> Self {
        Self {
            height: c.height.0,
            view: c.view.0,
            proposer: c.proposer.0,
            epoch: c.epoch.0,
            epoch_start_view: c.epoch_start_view.0,
            validator_set: Some(pb::ValidatorSet::from(&c.validator_set)),
            vote_extensions: c
                .vote_extensions
                .iter()
                .map(|(id, data)| pb::VoteExtension {
                    validator_id: id.0,
                    data: data.clone(),
                })
                .collect(),
            timestamp: c.timestamp,
        }
    }
}

impl From<OwnedBlockContext> for pb::BlockContext {
    fn from(c: OwnedBlockContext) -> Self {
        pb::BlockContext::from(&c)
    }
}

impl TryFrom<pb::BlockContext> for OwnedBlockContext {
    type Error = prost::DecodeError;

    fn try_from(c: pb::BlockContext) -> Result<Self, Self::Error> {
        Ok(Self {
            height: Height(c.height),
            view: ViewNumber(c.view),
            proposer: ValidatorId(c.proposer),
            epoch: EpochNumber(c.epoch),
            epoch_start_view: ViewNumber(c.epoch_start_view),
            validator_set: c
                .validator_set
                .ok_or_else(|| prost::DecodeError::new("missing validator_set"))?
                .try_into()?,
            timestamp: c.timestamp,
            vote_extensions: c
                .vote_extensions
                .into_iter()
                .map(|ve| (ValidatorId(ve.validator_id), ve.data))
                .collect(),
        })
    }
}

// ---- EquivocationProof ----

impl From<&EquivocationProof> for pb::EquivocationProof {
    fn from(e: &EquivocationProof) -> Self {
        Self {
            validator: e.validator.0,
            view: e.view.0,
            vote_type: match e.vote_type {
                VoteType::Vote => 0,
                VoteType::Vote2 => 1,
            },
            block_hash_a: e.block_hash_a.0.to_vec(),
            signature_a: e.signature_a.0.clone(),
            extension_a: e.extension_a.clone().unwrap_or_default(),
            has_extension_a: e.extension_a.is_some(),
            block_hash_b: e.block_hash_b.0.to_vec(),
            signature_b: e.signature_b.0.clone(),
            extension_b: e.extension_b.clone().unwrap_or_default(),
            has_extension_b: e.extension_b.is_some(),
            epoch: e.epoch.as_u64(),
        }
    }
}

impl From<EquivocationProof> for pb::EquivocationProof {
    fn from(e: EquivocationProof) -> Self {
        pb::EquivocationProof::from(&e)
    }
}

impl TryFrom<pb::EquivocationProof> for EquivocationProof {
    type Error = prost::DecodeError;
    fn try_from(e: pb::EquivocationProof) -> Result<Self, Self::Error> {
        Ok(Self {
            validator: ValidatorId(e.validator),
            view: ViewNumber(e.view),
            vote_type: match e.vote_type {
                0 => VoteType::Vote,
                1 => VoteType::Vote2,
                other => {
                    return Err(prost::DecodeError::new(format!(
                        "invalid vote_type: {other}"
                    )));
                }
            },
            epoch: EpochNumber(e.epoch),
            block_hash_a: bytes_to_hash(&e.block_hash_a)?,
            signature_a: bytes_to_signature(&e.signature_a)?,
            extension_a: e.has_extension_a.then_some(e.extension_a),
            block_hash_b: bytes_to_hash(&e.block_hash_b)?,
            signature_b: bytes_to_signature(&e.signature_b)?,
            extension_b: e.has_extension_b.then_some(e.extension_b),
        })
    }
}

// ---- ValidatorUpdate ----

impl From<&ValidatorUpdate> for pb::ValidatorUpdate {
    fn from(u: &ValidatorUpdate) -> Self {
        Self {
            id: u.id.0,
            public_key: u.public_key.0.clone(),
            power: u.power,
        }
    }
}

impl TryFrom<pb::ValidatorUpdate> for ValidatorUpdate {
    type Error = prost::DecodeError;

    fn try_from(u: pb::ValidatorUpdate) -> Result<Self, Self::Error> {
        Ok(Self {
            id: ValidatorId(u.id),
            public_key: bytes_to_public_key(&u.public_key)?,
            power: u.power,
        })
    }
}

// ---- EventAttribute ----

impl From<&EventAttribute> for pb::EventAttribute {
    fn from(a: &EventAttribute) -> Self {
        Self {
            key: a.key.clone(),
            value: a.value.clone(),
        }
    }
}

impl From<pb::EventAttribute> for EventAttribute {
    fn from(a: pb::EventAttribute) -> Self {
        Self {
            key: a.key,
            value: a.value,
        }
    }
}

// ---- Event ----

impl From<&Event> for pb::Event {
    fn from(e: &Event) -> Self {
        Self {
            r#type: e.r#type.clone(),
            attributes: e.attributes.iter().map(pb::EventAttribute::from).collect(),
        }
    }
}

impl From<pb::Event> for Event {
    fn from(e: pb::Event) -> Self {
        Self {
            r#type: e.r#type,
            attributes: e.attributes.into_iter().map(Into::into).collect(),
        }
    }
}

// ---- EndBlockResponse ----

impl From<&EndBlockResponse> for pb::EndBlockResponse {
    fn from(r: &EndBlockResponse) -> Self {
        Self {
            validator_updates: r
                .validator_updates
                .iter()
                .map(pb::ValidatorUpdate::from)
                .collect(),
            events: r.events.iter().map(pb::Event::from).collect(),
            app_hash: r.app_hash.0.to_vec(),
        }
    }
}

impl From<EndBlockResponse> for pb::EndBlockResponse {
    fn from(r: EndBlockResponse) -> Self {
        pb::EndBlockResponse::from(&r)
    }
}

impl TryFrom<pb::EndBlockResponse> for EndBlockResponse {
    type Error = prost::DecodeError;
    fn try_from(r: pb::EndBlockResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            validator_updates: r
                .validator_updates
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            events: r.events.into_iter().map(Into::into).collect(),
            app_hash: bytes_to_hash(&r.app_hash)?,
        })
    }
}

// ---- SnapshotInfo ----

impl From<&SnapshotInfo> for pb::SnapshotInfo {
    fn from(s: &SnapshotInfo) -> Self {
        Self {
            height: s.height.0,
            chunks: s.chunks,
            hash: s.hash.to_vec(),
        }
    }
}

impl TryFrom<pb::SnapshotInfo> for SnapshotInfo {
    type Error = prost::DecodeError;

    fn try_from(s: pb::SnapshotInfo) -> Result<Self, Self::Error> {
        Ok(Self {
            height: Height(s.height),
            chunks: s.chunks,
            hash: bytes_to_array_32("snapshot hash", &s.hash)?,
        })
    }
}

// ---- Helpers ----

fn bytes_to_hash(bytes: &[u8]) -> Result<BlockHash, prost::DecodeError> {
    Ok(BlockHash(bytes_to_array_32("block hash", bytes)?))
}

fn bytes_to_public_key(bytes: &[u8]) -> Result<PublicKey, prost::DecodeError> {
    let public_key = bytes_to_array_32("public key", bytes)?;
    Ok(PublicKey(public_key.to_vec()))
}

fn bytes_to_signature(bytes: &[u8]) -> Result<Signature, prost::DecodeError> {
    if bytes.len() != 64 {
        return Err(prost::DecodeError::new(format!(
            "signature: expected 64 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(Signature(bytes.to_vec()))
}

fn bytes_to_array_32(field: &str, bytes: &[u8]) -> Result<[u8; 32], prost::DecodeError> {
    bytes.try_into().map_err(|_| {
        prost::DecodeError::new(format!("{field}: expected 32 bytes, got {}", bytes.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_hash_lengths() {
        let block = pb::Block {
            height: 1,
            parent_hash: vec![],
            view: 0,
            proposer: 0,
            timestamp: 0,
            payload: vec![],
            hash: vec![0; 32],
            app_hash: vec![0; 32],
            evidence: vec![],
        };
        let err = Block::try_from(block).unwrap_err();
        assert!(err.to_string().contains("expected 32 bytes"));
    }

    #[test]
    fn rejects_malformed_public_keys() {
        let validator = pb::ValidatorInfo {
            id: 0,
            public_key: vec![0; 31],
            power: 1,
        };
        let err = ValidatorInfo::try_from(validator).unwrap_err();
        assert!(err.to_string().contains("public key"));
    }

    #[test]
    fn rejects_malformed_signatures() {
        let proof = pb::EquivocationProof {
            validator: 0,
            view: 0,
            vote_type: 0,
            block_hash_a: vec![0; 32],
            signature_a: vec![0; 63],
            block_hash_b: vec![1; 32],
            signature_b: vec![0; 64],
            epoch: 0,
            extension_a: vec![],
            extension_b: vec![],
            has_extension_a: false,
            has_extension_b: false,
        };
        let err = EquivocationProof::try_from(proof).unwrap_err();
        assert!(err.to_string().contains("signature"));
    }
}
