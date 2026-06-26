# Rust Library

The `crabdb` crate exports a library API as well as the CLI binary.

## Main Exports

```rust
pub use db::{CrabDb, InitImportMode};
pub use error::{Error, Result};
pub use ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};
pub use model::*;
pub use ::prolly;
```

It also exports:

```rust
pub mod prolly_tree {
    pub use ::prolly::*;
}
```

And a prelude:

```rust
pub mod prelude {
    pub use crate::{Actor, CrabDb, Error, InitImportMode, PatchDocument, Result};
    pub use ::prolly::{Config, MemStore, Prolly, Store, Tree};
}
```

## Common Entry Points

Use `CrabDb::init`, `CrabDb::init_with_text_policy`, `CrabDb::open`, or discovery/open methods to create a handle, then call typed methods for record, status, agent lifecycle, sessions, patches, merge queue, conflicts, backups, and maintenance.

## Data Types

Reports and model types are serializable with Serde. The CLI, HTTP API, MCP tools, and Rust API share many of the same report structs.

## Code Facts Used

- Library exports: `crates/crabdb/src/lib.rs`
- Public methods: `crates/crabdb/src/db`
- Public models: `crates/crabdb/src/model`
- Test: `prolly_is_importable_through_crabdb_namespaces`

