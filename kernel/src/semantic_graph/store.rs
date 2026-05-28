//! Static Graph Store implementation for Phase 8 Semantic Graph Filesystem

use super::types::{
    GraphNode, ObjectId, ObjectType, RelType, Edge, Property, QueryPredicate,
    OBJECT_ID_NULL, MAX_PROPERTIES, MAX_EDGES, MAX_STR_LEN,
};
use spin::Mutex;
use core::sync::atomic::{AtomicU64, Ordering};

pub const MAX_GRAPH_NODES: usize = 256;

pub struct StoreState {
    pub nodes: [GraphNode; MAX_GRAPH_NODES],
}

impl StoreState {
    pub const fn new() -> Self {
        const DEFAULT_NODE: GraphNode = GraphNode {
            id: OBJECT_ID_NULL,
            object_type: ObjectType::Blob,
            vmo_handle: 0,
            blob_size: 0,
            properties: super::types::PropertyStore {
                count: 0,
                store: [Property { key: [0; MAX_STR_LEN], val: [0; MAX_STR_LEN] }; MAX_PROPERTIES],
            },
            edges: super::types::EdgeList {
                count: 0,
                store: [Edge { relationship: RelType::RelatedTo, target: OBJECT_ID_NULL }; MAX_EDGES],
            },
            ref_count: 0,
            owner_pid: 0,
            allocated: false,
        };
        
        Self {
            nodes: [DEFAULT_NODE; MAX_GRAPH_NODES],
        }
    }
}

impl Default for StoreState {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreState {
    /// Allocate a new node in the store
    pub fn alloc_node(&mut self, object_type: ObjectType, owner_pid: u32) -> Result<ObjectId, &'static str> {
        for slot in self.nodes.iter_mut() {
            if !slot.allocated {
                let id = NEXT_OBJECT_ID.fetch_add(1, Ordering::SeqCst);
                slot.id = id;
                slot.object_type = object_type;
                slot.vmo_handle = 0;
                slot.blob_size = 0;
                slot.properties = Default::default();
                slot.edges = Default::default();
                slot.ref_count = 1;
                slot.owner_pid = owner_pid;
                slot.allocated = true;
                return Ok(id);
            }
        }
        Err("GraphStore is full")
    }

    /// Retrieve node index by ObjectId
    fn find_index(&self, id: ObjectId) -> Option<usize> {
        if id == OBJECT_ID_NULL {
            return None;
        }
        for (idx, slot) in self.nodes.iter().enumerate() {
            if slot.allocated && slot.id == id {
                return Some(idx);
            }
        }
        None
    }

    /// Check if node exists
    pub fn exists(&self, id: ObjectId) -> bool {
        self.find_index(id).is_some()
    }

    /// Add a directed relationship edge from src to target
    pub fn add_edge(&mut self, src_id: ObjectId, relationship: RelType, target_id: ObjectId) -> Result<(), &'static str> {
        if !self.exists(target_id) {
            return Err("Target node does not exist");
        }
        
        if let Some(src_idx) = self.find_index(src_id) {
            let src_node = &mut self.nodes[src_idx];
            if src_node.edges.count >= MAX_EDGES {
                return Err("Maximum edge count exceeded");
            }
            
            // Check for duplicate edge
            for e in 0..src_node.edges.count {
                let edge = &src_node.edges.store[e];
                if edge.relationship == relationship && edge.target == target_id {
                    return Ok(()); // Edge already exists
                }
            }
            
            let count = src_node.edges.count;
            src_node.edges.store[count] = Edge { relationship, target: target_id };
            src_node.edges.count += 1;
            Ok(())
        } else {
            Err("Source node does not exist")
        }
    }

    /// Execute a search query over all allocated nodes matching predicates
    pub fn query(&self, predicate: &QueryPredicate, out_buf: &mut [ObjectId]) -> usize {
        let mut count = 0;
        for node in self.nodes.iter() {
            if !node.allocated {
                continue;
            }
            
            // 1. Filter by object type
            if predicate.has_object_type && node.object_type != predicate.object_type {
                continue;
            }
            
            // 2. Filter by property key-value
            if predicate.has_property {
                let mut prop_match = false;
                for p in 0..node.properties.count {
                    let prop = &node.properties.store[p];
                    if prop.key == predicate.property_key && prop.val == predicate.property_val {
                        prop_match = true;
                        break;
                    }
                }
                if !prop_match {
                    continue;
                }
            }
            
            // 3. Filter by edge relationship & target
            if predicate.has_edge {
                let mut edge_match = false;
                for e in 0..node.edges.count {
                    let edge = &node.edges.store[e];
                    if edge.relationship == predicate.edge_type && edge.target == predicate.edge_target {
                        edge_match = true;
                        break;
                    }
                }
                if !edge_match {
                    continue;
                }
            }
            
            // All filters match
            if count < out_buf.len() {
                out_buf[count] = node.id;
                count += 1;
            } else {
                break; // Output buffer is full
            }
        }
        count
    }
}

pub static GRAPH_STORE: Mutex<StoreState> = Mutex::new(StoreState::new());
static NEXT_OBJECT_ID: AtomicU64 = AtomicU64::new(1);

/// Thread-safe helper to mutate a specific node
pub fn with_node_mut<F, R>(id: ObjectId, f: F) -> Option<R>
where
    F: FnOnce(&mut GraphNode) -> R,
{
    let mut store = GRAPH_STORE.lock();
    if let Some(idx) = store.find_index(id) {
        Some(f(&mut store.nodes[idx]))
    } else {
        None
    }
}

/// Thread-safe helper to view a specific node
pub fn with_node<F, R>(id: ObjectId, f: F) -> Option<R>
where
    F: FnOnce(&GraphNode) -> R,
{
    let store = GRAPH_STORE.lock();
    if let Some(idx) = store.find_index(id) {
        Some(f(&store.nodes[idx]))
    } else {
        None
    }
}
