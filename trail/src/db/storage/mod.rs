use super::*;
use crate::db::util::*;

mod content;
mod diff;
mod file_build;
mod files;
mod git;
mod ids;
mod lane_gates;
mod lane_lookup;
mod lane_runs;
mod lanes;
mod lifecycle;
mod line_changes;
mod manifest;
mod memory;
mod objects;
mod patches;
mod query;
mod record_selection;
mod refs;
mod root_diff;
mod schema;
mod validation;
mod worktree_index;
mod worktree_scan;

#[cfg(debug_assertions)]
pub(crate) use git::{
    install_git_qualification_after_c2_hook, install_git_qualification_after_porcelain_hook,
};
#[cfg(any(test, debug_assertions))]
pub(crate) use schema::{
    clear_schema_v19_migration_failure, clear_schema_v20_migration_failure,
    create_schema_v18_fixture_for_test, install_schema_v19_migration_failure,
    install_schema_v20_migration_failure, SchemaV19MigrationBoundary, SchemaV20MigrationBoundary,
};
pub(crate) use schema::{
    migrate_schema_v18_to_v19, migrate_schema_v19_to_v20, validate_prolly_sqlite_schema_v18,
    validate_schema_v18_for_migration, validate_schema_v19_for_migration, validate_schema_v20,
};
pub(crate) use worktree_index::{
    file_kind_from_index, PinnedWorktreeRoot, ReconciliationDirectory, ReconciliationFile,
    ReconciliationScanEntry,
};
pub(crate) use worktree_scan::{observed_exact_paths_for_candidates, ObservedPathKind};
