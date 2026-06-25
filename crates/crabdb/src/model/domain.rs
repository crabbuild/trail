use serde::{Deserialize, Serialize};

use crate::ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};

include!("domain/config.rs");
include!("domain/objects.rs");
include!("domain/operations.rs");
