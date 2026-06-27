#![recursion_limit = "256"]

//! CrabDB core library.
//!
//! CrabDB is a local-first operation database for code and text worktrees. It
//! records meaningful operations, preserves stable file and line identity, and
//! gives humans and coding lanes a safe branch/provenance layer above Git.

pub mod db;
pub mod error;
pub mod ids;
pub mod mcp;
pub mod model;
pub mod server;

pub use db::{CrabDb, InitImportMode};
pub use error::{Error, Result};
pub use ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};
pub use model::*;

/// Re-export the prolly crate as a CrabDB module namespace.
pub use ::prolly;

/// Compatibility module for callers that prefer the explicit prolly-tree name.
pub mod prolly_tree {
    pub use ::prolly::*;
}

/// Common imports for CrabDB consumers.
pub mod prelude {
    pub use crate::{Actor, CrabDb, Error, InitImportMode, PatchDocument, Result};
    pub use ::prolly::{Config, MemStore, Prolly, Store, Tree};
}
