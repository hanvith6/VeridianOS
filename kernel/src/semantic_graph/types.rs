//! Core Data Model Types for VeridianOS Semantic Graph Filesystem (Phase 8)

pub type ObjectId = u64;
pub const OBJECT_ID_NULL: ObjectId = 0;

pub const MAX_PROPERTIES: usize = 8;
pub const MAX_EDGES: usize = 16;
pub const MAX_STR_LEN: usize = 32;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Blob        = 0,  // Raw bytes (replaces files)
    Document    = 1,  // Structured text/PDF/etc
    Image       = 2,  // Raster image data
    Code        = 3,  // Executable or source
    Config      = 4,  // Key-value configuration
    Contact     = 5,  // Person/org entity
    Project     = 6,  // Grouping concept
    Session     = 7,  // Login/auth session
    Agent       = 8,  // AI agent entity
    Custom      = 9,  // Application-defined types
}

impl ObjectType {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => ObjectType::Blob,
            1 => ObjectType::Document,
            2 => ObjectType::Image,
            3 => ObjectType::Code,
            4 => ObjectType::Config,
            5 => ObjectType::Contact,
            6 => ObjectType::Project,
            7 => ObjectType::Session,
            8 => ObjectType::Agent,
            _ => ObjectType::Custom,
        }
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelType {
    Contains      = 0,  // A contains B
    IsPartOf      = 1,  // A is part of B
    CreatedBy     = 2,  // A was created by B
    IsVersionOf   = 3,  // A is a version of B
    DependsOn     = 4,  // A depends on B
    RelatedTo     = 5,  // Generic association
    IsInvoiceFor  = 6,  // Document-specific
    BelongsTo     = 7,  // A belongs to project/group B
    Generates     = 8,  // Agent A generated B
    Custom        = 9,  // Application-defined
}

impl RelType {
    pub fn from_u16(val: u16) -> Self {
        match val {
            0 => RelType::Contains,
            1 => RelType::IsPartOf,
            2 => RelType::CreatedBy,
            3 => RelType::IsVersionOf,
            4 => RelType::DependsOn,
            5 => RelType::RelatedTo,
            6 => RelType::IsInvoiceFor,
            7 => RelType::BelongsTo,
            8 => RelType::Generates,
            _ => RelType::Custom,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub relationship: RelType,
    pub target:       ObjectId,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Property {
    pub key: [u8; MAX_STR_LEN],
    pub val: [u8; MAX_STR_LEN],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PropertyStore {
    pub count: usize,
    pub store: [Property; MAX_PROPERTIES],
}

impl Default for PropertyStore {
    fn default() -> Self {
        Self {
            count: 0,
            store: [Property { key: [0; MAX_STR_LEN], val: [0; MAX_STR_LEN] }; MAX_PROPERTIES],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EdgeList {
    pub count: usize,
    pub store: [Edge; MAX_EDGES],
}

impl Default for EdgeList {
    fn default() -> Self {
        Self {
            count: 0,
            store: [Edge { relationship: RelType::RelatedTo, target: OBJECT_ID_NULL }; MAX_EDGES],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GraphNode {
    pub id:          ObjectId,
    pub object_type: ObjectType,
    pub vmo_handle:  usize,          // Handle to VMO containing blob data (0 = none)
    pub blob_size:   usize,          // Size of blob in bytes
    pub properties:  PropertyStore,  // Up to 8 key-value string pairs
    pub edges:       EdgeList,       // Up to 16 outgoing edges
    pub ref_count:   u32,            // Reference count
    pub owner_pid:   u32,            // Creating process PID
    pub allocated:   bool,           // Slot allocation flag
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct QueryPredicate {
    pub has_object_type: bool,
    pub object_type: ObjectType,
    
    pub has_property: bool,
    pub property_key: [u8; MAX_STR_LEN],
    pub property_val: [u8; MAX_STR_LEN],
    
    pub has_edge: bool,
    pub edge_type: RelType,
    pub edge_target: ObjectId,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PropertiesInit {
    pub count: usize,
    pub keys: [[u8; MAX_STR_LEN]; MAX_PROPERTIES],
    pub values: [[u8; MAX_STR_LEN]; MAX_PROPERTIES],
}
