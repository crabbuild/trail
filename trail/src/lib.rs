#![recursion_limit = "512"]

//! Trail core library.
//!
//! Trail is a local-first operation database for code and text worktrees. It
//! records meaningful operations, preserves stable file and line identity, and
//! gives humans and coding lanes a safe branch/provenance layer above Git.

pub mod acp;
pub mod agent_hooks;
pub mod db;
pub mod error;
pub mod ids;
pub mod mcp;
pub mod model;
pub mod server;

pub use db::{InitImportMode, Trail};
pub use error::{Error, Result};
pub use ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};
pub use model::*;

#[cfg(debug_assertions)]
#[doc(hidden)]
pub mod test_support {
    pub fn changed_path_reconciliation_oracle() -> std::result::Result<(), String> {
        crate::db::run_oracle()
    }

    pub fn changed_path_reconciliation_races() -> std::result::Result<(), String> {
        crate::db::run_races()
    }

    pub fn changed_path_reconciliation_callback_spool() -> std::result::Result<(), String> {
        crate::db::run_callback_spool()
    }

    pub fn changed_path_intent_acknowledgement_race() -> std::result::Result<(), String> {
        crate::db::run_acknowledgement_race()
    }

    pub fn changed_path_intent_gc_root_lifecycle() -> std::result::Result<(), String> {
        crate::db::run_gc_root_lifecycle()
    }

    pub fn changed_path_intent_crash_matrix() -> std::result::Result<(), String> {
        crate::db::run_crash_matrix()
    }

    pub fn changed_path_backup_restore_rotation() -> std::result::Result<(), String> {
        crate::db::run_backup_restore_rotation()
    }

    pub fn changed_path_qualified_proof_revalidation() -> std::result::Result<(), String> {
        crate::db::run_qualified_proof_revalidation()
    }

    pub fn changed_path_ambiguous_recovery_gate() -> std::result::Result<(), String> {
        crate::db::run_ambiguous_recovery_gate()
    }

    pub fn changed_path_backup_overwrite_rollback() -> std::result::Result<(), String> {
        crate::db::run_backup_overwrite_rollback()
    }

    pub fn changed_path_retirement_barrier() -> std::result::Result<(), String> {
        crate::db::run_retirement_barrier()
    }

    pub fn changed_path_lane_deletion_retirement() -> std::result::Result<(), String> {
        crate::db::run_lane_deletion_retirement()
    }
}

/// Re-export the prolly crate as a Trail module namespace.
pub use ::prolly;

/// Compatibility module for callers that prefer the explicit prolly-tree name.
pub mod prolly_tree {
    pub use ::prolly::*;
}

/// Common imports for Trail consumers.
pub mod prelude {
    pub use crate::{Actor, Error, InitImportMode, PatchDocument, Result, Trail};
    pub use ::prolly::{Config, MemStore, Prolly, Store, Tree};
}
