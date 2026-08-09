use serde::{Deserialize, Serialize};

use crate::ids::{
    AnchorId, ArtifactBlobId, ArtifactChunkId, ArtifactChunkListId, ArtifactDesiredKeyV2,
    ArtifactEnvelopeId, ArtifactFileId, ArtifactQuarantineId, ArtifactTreeId, ChangeId, FileId,
    LineId, MessageId, ObjectId, WorkspaceId,
};

include!("domain/config.rs");
include!("domain/artifacts.rs");
include!("domain/memory.rs");
include!("domain/objects.rs");
include!("domain/operations.rs");
