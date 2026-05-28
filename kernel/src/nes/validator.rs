//! Topological DAG cycle validator for the Neural Execution Subsystem (NES).

use super::graph::{TaskGraph, MAX_NODES_PER_GRAPH};

/// Traverses the task graph to verify no recursive execution cycles exist.
pub fn validate_dag(graph: &TaskGraph) -> Result<(), &'static str> {
    // 0 = Unvisited, 1 = Visiting (on stack), 2 = Visited (finished)
    let mut state = [0u8; MAX_NODES_PER_GRAPH];

    for i in 0..graph.num_nodes {
        if state[i] == 0
            && has_cycle_dfs(i, graph, &mut state) {
                return Err("Cycle detected inside task graph");
            }
    }
    Ok(())
}

fn has_cycle_dfs(node_idx: usize, graph: &TaskGraph, state: &mut [u8]) -> bool {
    state[node_idx] = 1; // Mark as visiting

    // Traverse all nodes that depend on this node
    let count = graph.successor_counts[node_idx];
    for s in 0..count {
        let succ_id = graph.node_successors[node_idx][s];
        
        // Find index of successor node
        if let Some(succ_idx) = (0..graph.num_nodes).find(|&i| graph.nodes[i].node_id == succ_id) {
            if state[succ_idx] == 1 {
                return true; // Visited visiting node -> Backedge (Cycle!)
            }
            if state[succ_idx] == 0
                && has_cycle_dfs(succ_idx, graph, state) {
                    return true;
                }
        }
    }

    state[node_idx] = 2; // Mark as visited
    false
}
