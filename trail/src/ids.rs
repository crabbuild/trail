use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

static NONCE: AtomicU64 = AtomicU64::new(1);

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
        Self(format!("wk_{}", short_hash(seed, 16)))
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
        Self(format!("ch_{}", hex::encode(hasher.finalize())))
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
}

impl MessageId {
    pub fn new(change_id: &ChangeId, role: &str, body: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(change_id.0.as_bytes());
        hasher.update(role.as_bytes());
        hasher.update(body.as_bytes());
        Self(format!("msg_{}", hex::encode(hasher.finalize())))
    }
}

impl AnchorId {
    pub fn new(file_id: &FileId, line_id: &LineId, label: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(file_id.encode_key());
        hasher.update(line_id.encode_key());
        hasher.update(label.as_bytes());
        Self(format!("anc_{}", hex::encode(hasher.finalize())))
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
        Self(format!("obj_{}", hex::encode(hasher.finalize())))
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
    let digest = if let Some(hex_id) = change_id.strip_prefix("ch_") {
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

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
