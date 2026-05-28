//! VeridianOS Semantic FS Verification Program
//!
//! Creates a document node with properties and a blob node, connects them via an edge,
//! writes content into the blob's VMO buffer, and queries the database to verify everything works.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[inline(always)]
pub fn syscall5(id: usize, arg0: usize, arg1: usize, arg2: usize, arg3: usize, arg4: usize) -> isize {
    let ret;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") id,
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            in("a3") arg3,
            in("a4") arg4,
            lateout("a0") ret,
        );
    }
    ret
}

const SYS_WRITE: usize = 1;
const SYS_EXIT: usize = 2;
const SYS_NODE_CREATE: usize = 60;
const SYS_EDGE_ADD: usize = 61;
const SYS_NODE_WRITE: usize = 62;
const SYS_GRAPH_QUERY: usize = 63;

fn print(s: &str) {
    syscall5(SYS_WRITE, s.as_ptr() as usize, s.len(), 0, 0, 0);
}

pub const MAX_PROPERTIES: usize = 8;
pub const MAX_EDGES: usize = 16;
pub const MAX_STR_LEN: usize = 32;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Blob        = 0,
    Document    = 1,
    Image       = 2,
    Code        = 3,
    Config      = 4,
    Contact     = 5,
    Project     = 6,
    Session     = 7,
    Agent       = 8,
    Custom      = 9,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelType {
    Contains      = 0,
    IsPartOf      = 1,
    CreatedBy     = 2,
    IsVersionOf   = 3,
    DependsOn     = 4,
    RelatedTo     = 5,
    IsInvoiceFor  = 6,
    BelongsTo     = 7,
    Generates     = 8,
    Custom        = 9,
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
    pub edge_target: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PropertiesInit {
    pub count: usize,
    pub keys: [[u8; MAX_STR_LEN]; MAX_PROPERTIES],
    pub values: [[u8; MAX_STR_LEN]; MAX_PROPERTIES],
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    print("[USER] Starting Semantic Knowledge Graph Filesystem Verification program...\n");

    // 1. Create properties for the document node: title="OS_Plan", quarter="Q2"
    let mut keys = [[0u8; MAX_STR_LEN]; MAX_PROPERTIES];
    let mut values = [[0u8; MAX_STR_LEN]; MAX_PROPERTIES];

    let k1 = b"title";
    let v1 = b"OS_Plan";
    keys[0][..k1.len()].copy_from_slice(k1);
    values[0][..v1.len()].copy_from_slice(v1);

    let k2 = b"quarter";
    let v2 = b"Q2";
    keys[1][..k2.len()].copy_from_slice(k2);
    values[1][..v2.len()].copy_from_slice(v2);

    let props_init = PropertiesInit {
        count: 2,
        keys,
        values,
    };

    // Create Document node
    let doc_handle_ret = syscall5(
        SYS_NODE_CREATE,
        1, // ObjectType::Document
        0, // blob_size = 0
        &props_init as *const PropertiesInit as usize,
        0,
        0,
    );

    if doc_handle_ret < 0 {
        print("[USER] Error creating document node!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    let doc_handle = doc_handle_ret as usize;
    print("[USER] Created Document node capability successfully.\n");

    // Create Blob node (size = 32 bytes)
    let blob_handle_ret = syscall5(
        SYS_NODE_CREATE,
        0,  // ObjectType::Blob
        32, // blob_size = 32
        0,  // no properties init
        0,
        0,
    );

    if blob_handle_ret < 0 {
        print("[USER] Error creating blob node!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    let blob_handle = blob_handle_ret as usize;
    print("[USER] Created Blob node capability successfully.\n");

    // Write text to the Blob node VMO
    let content = b"VeridianOS Phase 8 works great!!";
    let write_ret = syscall5(
        SYS_NODE_WRITE,
        blob_handle,
        content.as_ptr() as usize,
        content.len(),
        0, // offset
        0,
    );

    if write_ret < 0 {
        print("[USER] Error writing to blob node!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    print("[USER] Wrote text content into Blob node VMO successfully.\n");

    // 2. Query the Document node's ObjectId using its properties
    // Query: ObjectType == Document && title == "OS_Plan"
    let mut q_key = [0u8; MAX_STR_LEN];
    let mut q_val = [0u8; MAX_STR_LEN];
    q_key[..k1.len()].copy_from_slice(k1);
    q_val[..v1.len()].copy_from_slice(v1);

    let predicate = QueryPredicate {
        has_object_type: true,
        object_type: ObjectType::Document,
        has_property: true,
        property_key: q_key,
        property_val: q_val,
        has_edge: false,
        edge_type: RelType::RelatedTo,
        edge_target: 0,
    };

    let mut query_results = [0u64; 8];
    let query_ret = syscall5(
        SYS_GRAPH_QUERY,
        &predicate as *const QueryPredicate as usize,
        query_results.as_mut_ptr() as usize,
        8, // max results
        0,
        0,
    );

    if query_ret <= 0 {
        print("[USER] Query found no matching nodes!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    let matched_doc_id = query_results[0];
    print("[USER] Successfully queried Document node ID by properties.\n");

    // 3. Connect Document node to Blob node via an edge (Document -[Contains]-> Blob)
    // First we need to know the Blob node's ObjectId.
    // Let's query for the Blob node
    let predicate_blob = QueryPredicate {
        has_object_type: true,
        object_type: ObjectType::Blob,
        has_property: false,
        property_key: [0; MAX_STR_LEN],
        property_val: [0; MAX_STR_LEN],
        has_edge: false,
        edge_type: RelType::RelatedTo,
        edge_target: 0,
    };

    let mut query_results_blob = [0u64; 8];
    let query_blob_ret = syscall5(
        SYS_GRAPH_QUERY,
        &predicate_blob as *const QueryPredicate as usize,
        query_results_blob.as_mut_ptr() as usize,
        8,
        0,
        0,
    );

    if query_blob_ret <= 0 {
        print("[USER] Query found no Blob nodes!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    let matched_blob_id = query_results_blob[0];

    // Add Edge
    let edge_ret = syscall5(
        SYS_EDGE_ADD,
        doc_handle,
        0, // RelType::Contains (u16)
        matched_blob_id as usize,
        0,
        0,
    );

    if edge_ret < 0 {
        print("[USER] Error adding edge from Document to Blob!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }
    print("[USER] Added directed edge (Document -[Contains]-> Blob) successfully.\n");

    // 4. Query using the edge relation: find node of type Document containing the Blob ID
    let predicate_edge = QueryPredicate {
        has_object_type: true,
        object_type: ObjectType::Document,
        has_property: false,
        property_key: [0; MAX_STR_LEN],
        property_val: [0; MAX_STR_LEN],
        has_edge: true,
        edge_type: RelType::Contains,
        edge_target: matched_blob_id,
    };

    let mut edge_query_results = [0u64; 8];
    let edge_query_ret = syscall5(
        SYS_GRAPH_QUERY,
        &predicate_edge as *const QueryPredicate as usize,
        edge_query_results.as_mut_ptr() as usize,
        8,
        0,
        0,
    );

    if edge_query_ret <= 0 || edge_query_results[0] != matched_doc_id {
        print("[USER] Relational query verification failed!\n");
        syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    }

    print("[USER] Relational edge query verification SUCCESS!\n");
    print("[USER] Semantic Knowledge Graph Filesystem Verification SUCCESS!\n");

    syscall5(SYS_EXIT, 0, 0, 0, 0, 0);
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall5(SYS_EXIT, 1, 0, 0, 0, 0);
    loop {}
}
