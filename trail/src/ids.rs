use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

static NONCE: AtomicU64 = AtomicU64::new(1);

pub(crate) const WORKSPACE_ID_PREFIX: &str = "workspace_";
pub(crate) const CHANGE_ID_PREFIX: &str = "change_";
pub(crate) const OBJECT_ID_PREFIX: &str = "object_";
pub(crate) const MESSAGE_ID_PREFIX: &str = "message_";
pub(crate) const ANCHOR_ID_PREFIX: &str = "anchor_";
pub(crate) const CHECKPOINT_ALIAS_PREFIX: &str = "checkpoint_";
pub(crate) const LINE_ALIAS_PREFIX: &str = "line_";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChangeId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileId {
    pub origin_change: ChangeId,
    pub local_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LineId {
    pub origin_change: ChangeId,
    pub local_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AnchorId(pub String);

impl WorkspaceId {
    pub fn new(seed: &[u8]) -> Self {
        Self(format!("{WORKSPACE_ID_PREFIX}{}", short_hash(seed, 16)))
    }
}

impl ChangeId {
    pub fn allocate(workspace: &WorkspaceId, actor: &str, lamport: i64, hint: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let mut hasher = Sha256::new();
        hasher.update(workspace.0.as_bytes());
        hasher.update(actor.as_bytes());
        hasher.update(lamport.to_be_bytes());
        hasher.update(now.to_be_bytes());
        hasher.update(nonce.to_be_bytes());
        hasher.update(hint.as_bytes());
        Self(format!(
            "{CHANGE_ID_PREFIX}{}",
            hex::encode(hasher.finalize())
        ))
    }

    pub fn checkpoint_alias(&self) -> String {
        format!(
            "{CHECKPOINT_ALIAS_PREFIX}{}",
            self.0.strip_prefix(CHANGE_ID_PREFIX).unwrap_or(&self.0)
        )
    }

    pub fn from_checkpoint_alias(value: &str) -> Option<Self> {
        value
            .strip_prefix(CHECKPOINT_ALIAS_PREFIX)
            .map(|hash| Self(format!("{CHANGE_ID_PREFIX}{hash}")))
    }
}

impl FileId {
    pub fn new(origin_change: ChangeId, local_seq: u64) -> Self {
        Self {
            origin_change,
            local_seq,
        }
    }

    pub fn encode_key(&self) -> Vec<u8> {
        encode_compound_id(&self.origin_change.0, self.local_seq)
    }
}

impl LineId {
    pub fn new(origin_change: ChangeId, local_seq: u64) -> Self {
        Self {
            origin_change,
            local_seq,
        }
    }

    pub fn encode_key(&self) -> Vec<u8> {
        encode_compound_id(&self.origin_change.0, self.local_seq)
    }

    pub fn alias(&self) -> String {
        let origin = self
            .origin_change
            .0
            .strip_prefix(CHANGE_ID_PREFIX)
            .unwrap_or(&self.origin_change.0);
        format!("{LINE_ALIAS_PREFIX}{origin}:{}", self.local_seq)
    }

    pub fn from_alias(value: &str) -> Option<Self> {
        let (origin, local_seq) = value.rsplit_once(':')?;
        let origin = origin.strip_prefix(LINE_ALIAS_PREFIX)?;
        let local_seq = local_seq.parse::<u64>().ok()?;
        Some(Self::new(
            ChangeId(format!("{CHANGE_ID_PREFIX}{origin}")),
            local_seq,
        ))
    }
}

impl MessageId {
    pub fn new(change_id: &ChangeId, role: &str, body: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(change_id.0.as_bytes());
        hasher.update(role.as_bytes());
        hasher.update(body.as_bytes());
        Self(format!(
            "{MESSAGE_ID_PREFIX}{}",
            hex::encode(hasher.finalize())
        ))
    }
}

impl AnchorId {
    pub fn new(file_id: &FileId, line_id: &LineId, label: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(file_id.encode_key());
        hasher.update(line_id.encode_key());
        hasher.update(label.as_bytes());
        Self(format!(
            "{ANCHOR_ID_PREFIX}{}",
            hex::encode(hasher.finalize())
        ))
    }
}

impl ObjectId {
    pub fn for_bytes(kind: &str, version: u16, bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"TRAIL");
        hasher.update(kind.as_bytes());
        hasher.update(version.to_le_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Self(format!(
            "{OBJECT_ID_PREFIX}{}",
            hex::encode(hasher.finalize())
        ))
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for ChangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn short_hash(seed: &[u8], bytes: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed);
    let digest = hasher.finalize();
    hex::encode(&digest[..bytes.min(digest.len())])
}

fn encode_compound_id(change_id: &str, local_seq: u64) -> Vec<u8> {
    let digest = if let Some(hex_id) = change_id_hash(change_id) {
        hex::decode(hex_id).unwrap_or_else(|_| Sha256::digest(change_id.as_bytes()).to_vec())
    } else {
        Sha256::digest(change_id.as_bytes()).to_vec()
    };
    let mut out = Vec::with_capacity(40);
    out.extend_from_slice(&digest[..32.min(digest.len())]);
    if out.len() < 32 {
        out.resize(32, 0);
    }
    out.extend_from_slice(&local_seq.to_be_bytes());
    out
}

pub(crate) fn is_change_id(value: &str) -> bool {
    value.starts_with(CHANGE_ID_PREFIX)
}

pub(crate) fn change_id_hash(value: &str) -> Option<&str> {
    value.strip_prefix(CHANGE_ID_PREFIX)
}

pub(crate) fn is_object_id(value: &str) -> bool {
    value.starts_with(OBJECT_ID_PREFIX)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_use_full_entity_prefixes() {
        let workspace = WorkspaceId::new(b"workspace");
        let change = ChangeId::allocate(&workspace, "actor", 1, "test");
        let object = ObjectId::for_bytes("Blob", 1, b"content");
        let message = MessageId::new(&change, "assistant", "done");
        let file = FileId::new(change.clone(), 1);
        let line = LineId::new(change.clone(), 2);
        let anchor = AnchorId::new(&file, &line, "example");

        assert!(workspace.0.starts_with(WORKSPACE_ID_PREFIX));
        assert!(change.0.starts_with(CHANGE_ID_PREFIX));
        assert!(object.0.starts_with(OBJECT_ID_PREFIX));
        assert!(message.0.starts_with(MESSAGE_ID_PREFIX));
        assert!(anchor.0.starts_with(ANCHOR_ID_PREFIX));
        assert_eq!(
            ChangeId::from_checkpoint_alias(&change.checkpoint_alias()),
            Some(change.clone())
        );
        assert_eq!(LineId::from_alias(&line.alias()), Some(line));
    }

    #[test]
    fn canonical_prefixes_are_recognized() {
        let digest = "ab".repeat(32);
        assert!(is_change_id(&format!("change_{digest}")));
        assert!(is_object_id("object_current"));
        assert!(!is_change_id(&format!("ch_{digest}")));
        assert!(!is_object_id("obj_legacy"));
    }
}
