//! CrabDB core library.
//!
//! CrabDB is a local-first operation database for code and text worktrees. The
//! current workspace starts with the prolly tree storage primitive as the first
//! importable building block.

/// Re-export the prolly crate as a CrabDB module namespace.
pub use ::prolly;

/// Compatibility module for callers that prefer the explicit prolly-tree name.
pub mod prolly_tree {
    pub use ::prolly::*;
}

/// Common imports for early CrabDB consumers.
pub mod prelude {
    pub use ::prolly::{Config, MemStore, Prolly, Store, Tree};
}
