//! Filesystem Module for VeridianOS
//!
//! Provides a thin abstraction over in-kernel file storage.
//! In Phase 6, the only backing store is the InitRAMFS loaded from disk.

pub mod ramfs;

pub use ramfs::RamFs;
