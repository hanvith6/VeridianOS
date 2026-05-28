//! Semantic Knowledge Graph Filesystem (Phase 8)
//!
//! Replaces hierarchical byte-oriented directories with an entity-relationship
//! semantic graph database managed directly in kernel space.

pub mod types;
pub mod store;
pub mod syscalls;

pub use types::{
    ObjectId, ObjectType, RelType, Edge, Property, QueryPredicate, PropertiesInit,
    OBJECT_ID_NULL,
};
pub use store::{GRAPH_STORE, with_node, with_node_mut};
pub use syscalls::{sys_node_create, sys_edge_add, sys_node_write, sys_graph_query, sys_node_delete};

/// Initialize the semantic graph database
pub fn init() {
    crate::println!("[SEMANTIC_FS] Semantic Knowledge Graph Filesystem Initialized.");
}
