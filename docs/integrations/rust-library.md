# Rust Library

The `trail` crate exports a library API as well as the CLI binary.

## Main Exports

```rust
pub use db::{Trail, InitImportMode};
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
    pub use crate::{Actor, Trail, Error, InitImportMode, PatchDocument, Result};
    pub use ::prolly::{Config, MemStore, Prolly, Store, Tree};
}
```

## Common Entry Points

Use `Trail::init`, `Trail::init_with_text_policy`, `Trail::open`, or discovery/open methods to create a handle, then call typed methods for record, status, agent lifecycle, sessions, patches, merge queue, conflicts, backups, and maintenance.

## Data Types

Reports and model types are serializable with Serde. The CLI, HTTP API, MCP tools, and Rust API share many of the same report structs.

## Code Facts Used

- Library exports: `trail/src/lib.rs`
- Public methods: `trail/src/db`
- Public models: `trail/src/model`
- Test: `prolly_is_importable_through_trail_namespaces`

