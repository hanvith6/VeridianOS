//! System Call Identifier Numbers for VeridianOS
//!
//! System calls are triggered via the RISC-V `ecall` instruction.
//! The identifier number is placed in the `a7` register.
//!
//! References:
//! - RISC-V Supervisor Binary Interface (SBI) Specification (for ecall patterns)
//! - Linux RISC-V Syscall numbers mapping

/// Syscall: Write characters to the UART console.
/// Registers:
/// - `a7` = SYS_WRITE (1)
/// - `a0` = pointer to string buffer
/// - `a1` = length of string
pub const SYS_WRITE: usize = 1;

/// Syscall: Terminate the current process.
/// Registers:
/// - `a7` = SYS_EXIT (2)
/// - `a0` = exit status code
pub const SYS_EXIT: usize = 2;

/// Syscall: Close a capability handle.
/// Registers:
/// - `a7` = SYS_HANDLE_CLOSE (3)
/// - `a0` = handle ID to close
pub const SYS_HANDLE_CLOSE: usize = 3;

/// Syscall: Duplicate a capability handle with option to reduce rights.
/// Registers:
/// - `a7` = SYS_HANDLE_DUPLICATE (4)
/// - `a0` = source handle ID
/// - `a1` = rights bitmask to apply (0 to keep same)
pub const SYS_HANDLE_DUPLICATE: usize = 4;

/// Syscall: Create a new task graph.
/// Registers:
/// - `a7` = SYS_GRAPH_CREATE (50)
pub const SYS_GRAPH_CREATE: usize = 50;

/// Syscall: Add a node to a task graph.
/// Registers:
/// - `a7` = SYS_GRAPH_ADD_NODE (51)
/// - `a0` = graph handle ID
/// - `a1` = operation type (OpType)
/// - `a2` = pointer to NodeConfig struct
/// - `a3` = dependency count
/// - `a4` = pointer to dependency array
pub const SYS_GRAPH_ADD_NODE: usize = 51;

/// Syscall: Submit a task graph to a queue.
/// Registers:
/// - `a7` = SYS_GRAPH_SUBMIT (52)
/// - `a0` = graph handle ID
/// - `a1` = queue handle ID
pub const SYS_GRAPH_SUBMIT: usize = 52;

/// Syscall: Wait for a task graph to complete.
/// Registers:
/// - `a7` = SYS_GRAPH_WAIT (53)
/// - `a0` = graph handle ID
/// - `a1` = timeout in microseconds
pub const SYS_GRAPH_WAIT: usize = 53;

/// Syscall: Create a new semantic graph node.
pub const SYS_NODE_CREATE: usize = 60;

/// Syscall: Add a directed edge between nodes.
pub const SYS_EDGE_ADD: usize = 61;

/// Syscall: Write data into a node's blob VMO.
pub const SYS_NODE_WRITE: usize = 62;

/// Syscall: Query matching nodes in the semantic graph.
pub const SYS_GRAPH_QUERY: usize = 63;

// -----------------------------------------------------------------------
// Phase 9 — Agent Runtime Syscalls
// -----------------------------------------------------------------------

/// Syscall: Spawn a new kernel-tracked AI agent.
/// Registers:
/// - `a7` = SYS_AGENT_SPAWN (70)
/// - `a0` = parent_agent_id (u32)
/// - `a1` = ptr to intent string (null-padded, MAX_INTENT_LEN bytes)
/// - `a2` = intent length
pub const SYS_AGENT_SPAWN: usize = 70;

/// Syscall: Create a new IPC channel owned by an agent.
/// Registers:
/// - `a7` = SYS_CHANNEL_CREATE (71)
/// - `a0` = owner_agent_id
pub const SYS_CHANNEL_CREATE: usize = 71;

/// Syscall: Send a message to an IPC channel.
/// Registers:
/// - `a7` = SYS_CHANNEL_SEND (72)
/// - `a0` = channel_id
/// - `a1` = ptr to message payload
/// - `a2` = payload length (max 64)
pub const SYS_CHANNEL_SEND: usize = 72;

/// Syscall: Receive a message from an IPC channel.
/// Registers:
/// - `a7` = SYS_CHANNEL_RECV (73)
/// - `a0` = channel_id
/// - `a1` = ptr to output buffer (64 bytes)
/// - `a2` = ptr to output length (usize)
pub const SYS_CHANNEL_RECV: usize = 73;

/// Syscall: Query the state of an agent.
/// Registers:
/// - `a7` = SYS_AGENT_STATUS (74)
/// - `a0` = agent_id
/// - `a1` = ptr to output u8 (AgentState as u8)
pub const SYS_AGENT_STATUS: usize = 74;

/// Syscall: Configure or query scheduler policy statistics.
/// Registers:
/// - `a7` = SYS_POLICY_CONFIGURE (80)
/// - `a0` = op (0 = GET_STATS, 1 = SET_EXPLORATION, 2 = RESET_STATS)
/// - `a1` = arg1
/// - `a2` = arg2
pub const SYS_POLICY_CONFIGURE: usize = 80;

// -----------------------------------------------------------------------
// Phase 11 — Distributed Multi-Kernel Coherence Syscalls
// -----------------------------------------------------------------------

/// Syscall: Join a distributed kernel domain.
pub const SYS_DOMAIN_JOIN: usize = 90;

/// Syscall: List all live kernel domains.
pub const SYS_DOMAIN_LIST: usize = 91;

/// Syscall: Get status of a specific kernel domain.
pub const SYS_DOMAIN_STATUS: usize = 92;

/// Syscall: Dispatch a NES node to a remote domain.
pub const SYS_GRAPH_DISPATCH_REMOTE: usize = 93;

/// Syscall: Wait for a remote graph node to complete.
pub const SYS_GRAPH_WAIT_REMOTE: usize = 94;

/// Syscall: Abort a remote graph node.
pub const SYS_GRAPH_ABORT_REMOTE: usize = 95;

/// Syscall: Export a local capability handle to a target domain.
pub const SYS_CAP_EXPORT: usize = 96;

/// Syscall: Import a capability from global UID.
pub const SYS_CAP_IMPORT: usize = 97;

/// Syscall: Revoke a distributed capability globally.
pub const SYS_CAP_REVOKE_REMOTE: usize = 98;

/// Syscall: Make a semantic graph node globally replicated.
pub const SYS_SGF_REPLICATE_ENABLE: usize = 99;

/// Syscall: Query replication status of a semantic graph node.
pub const SYS_SGF_REPLICATE_QUERY: usize = 100;

/// Syscall: Query local Raft status.
pub const SYS_SGF_RAFT_STATUS: usize = 101;


