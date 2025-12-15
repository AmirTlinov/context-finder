use crate::error::Result;
use crate::types::{CodeGraph, RelationshipType};
use petgraph::algo::astar;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use std::collections::HashSet;

type SymbolRelation = (NodeIndex, RelationshipType);
type SymbolRelations = Vec<SymbolRelation>;

impl CodeGraph {
    /// Find all nodes that current node calls (outgoing Calls edges)
    #[must_use]
    pub fn get_callees(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph
            .edges(node)
            .filter(|e| matches!(e.weight().relationship, RelationshipType::Calls))
            .map(|e| e.target())
            .collect()
    }

    /// Find all nodes that call current node (incoming Calls edges)
    #[must_use]
    pub fn get_callers(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph
            .node_indices()
            .filter(|&idx| {
                self.graph.edges(idx).any(|e| {
                    e.target() == node && matches!(e.weight().relationship, RelationshipType::Calls)
                })
            })
            .collect()
    }

    /// Find all nodes that current node uses (outgoing Uses edges)
    #[must_use]
    pub fn get_dependencies(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph
            .edges(node)
            .filter(|e| matches!(e.weight().relationship, RelationshipType::Uses))
            .map(|e| e.target())
            .collect()
    }

    /// Find all nodes related to current node within given depth
    /// Returns (`NodeIndex`, distance, `relationship_path`)
    #[must_use]
    pub fn get_related_nodes(
        &self,
        node: NodeIndex,
        max_depth: usize,
    ) -> Vec<(NodeIndex, usize, Vec<RelationshipType>)> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue = vec![(node, 0, vec![])];

        while let Some((current, depth, path)) = queue.pop() {
            if depth > max_depth || visited.contains(&current) {
                continue;
            }

            visited.insert(current);

            if current != node {
                result.push((current, depth, path.clone()));
            }

            if depth < max_depth {
                // Explore neighbors
                for edge in self.graph.edges(current) {
                    let target = edge.target();
                    if !visited.contains(&target) {
                        let mut new_path = path.clone();
                        new_path.push(edge.weight().relationship);
                        queue.push((target, depth + 1, new_path));
                    }
                }
            }
        }

        result
    }

    /// Find shortest path between two nodes with full path reconstruction
    #[must_use]
    pub fn find_path(&self, from: NodeIndex, to: NodeIndex) -> Option<Vec<NodeIndex>> {
        #[allow(clippy::cast_possible_truncation)]
        let result = astar(
            &self.graph,
            from,
            |n| n == to,
            |e| e.weight().weight as i32,
            |_| 0, // No heuristic needed for correctness
        );

        result.map(|(_cost, path)| path)
    }

    /// Find path with relationship types between nodes
    #[must_use]
    pub fn find_path_with_edges(
        &self,
        from: NodeIndex,
        to: NodeIndex,
    ) -> Option<Vec<(NodeIndex, Option<RelationshipType>)>> {
        let path = self.find_path(from, to)?;

        let mut result = vec![(path[0], None)];
        for window in path.windows(2) {
            let edge = self.graph.find_edge(window[0], window[1]);
            let rel = edge.map(|e| self.graph[e].relationship);
            result.push((window[1], rel));
        }
        Some(result)
    }

    /// Get nodes by relationship type
    #[must_use]
    pub fn get_nodes_by_relationship(
        &self,
        node: NodeIndex,
        rel_type: RelationshipType,
    ) -> Vec<NodeIndex> {
        self.graph
            .edges(node)
            .filter(|e| e.weight().relationship == rel_type)
            .map(|e| e.target())
            .collect()
    }

    /// Get all related chunks for a symbol (for context assembly)
    pub fn get_context_for_symbol(
        &self,
        symbol_name: &str,
        max_depth: usize,
    ) -> Result<Vec<String>> {
        let node = self
            .find_node(symbol_name)
            .ok_or_else(|| crate::error::GraphError::NodeNotFound(symbol_name.to_string()))?;

        let related = self.get_related_nodes(node, max_depth);

        let mut chunk_ids = HashSet::new();

        // Add current node's chunk
        if let Some(node_data) = self.get_node(node) {
            chunk_ids.insert(node_data.chunk_id.clone());
        }

        // Add related nodes' chunks
        for (related_node, _dist, _path) in related {
            if let Some(node_data) = self.get_node(related_node) {
                chunk_ids.insert(node_data.chunk_id.clone());
            }
        }

        Ok(chunk_ids.into_iter().collect())
    }

    // ============================================================================
    // New methods for MCP tools: impact, trace, explain, overview
    // ============================================================================

    /// Get all usages of a node (all incoming edges of any relationship type)
    /// Used by: impact tool
    #[must_use]
    pub fn get_all_usages(&self, node: NodeIndex) -> Vec<(NodeIndex, RelationshipType)> {
        self.graph
            .edges_directed(node, Direction::Incoming)
            .map(|e| (e.source(), e.weight().relationship))
            .collect()
    }

    /// Get transitive usages up to given depth
    /// Used by: impact tool with depth > 1
    #[must_use]
    pub fn get_transitive_usages(
        &self,
        node: NodeIndex,
        max_depth: usize,
    ) -> Vec<(NodeIndex, usize, Vec<RelationshipType>)> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue = vec![(node, 0, vec![])];

        while let Some((current, depth, path)) = queue.pop() {
            if depth > max_depth || visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if current != node {
                result.push((current, depth, path.clone()));
            }

            if depth < max_depth {
                for edge in self.graph.edges_directed(current, Direction::Incoming) {
                    let source = edge.source();
                    if !visited.contains(&source) {
                        let mut new_path = path.clone();
                        new_path.push(edge.weight().relationship);
                        queue.push((source, depth + 1, new_path));
                    }
                }
            }
        }

        result
    }

    /// Find entry points - symbols with no callers (potential main/handler functions)
    /// Used by: overview tool
    #[must_use]
    pub fn find_entry_points(&self) -> Vec<NodeIndex> {
        self.graph
            .node_indices()
            .filter(|&n| {
                // Has no incoming Calls edges
                let no_callers = self.get_callers(n).is_empty();
                // Is a function or method (not a type/struct)
                let is_callable = self
                    .get_node(n)
                    .map(|nd| {
                        matches!(
                            nd.symbol.symbol_type,
                            crate::types::SymbolType::Function | crate::types::SymbolType::Method
                        )
                    })
                    .unwrap_or(false);
                no_callers && is_callable
            })
            .collect()
    }

    /// Get coupling score for a node (total edges in + out)
    /// Used by: overview tool for hotspots
    #[must_use]
    pub fn coupling_score(&self, node: NodeIndex) -> usize {
        let outgoing = self.graph.edges(node).count();
        let incoming = self.graph.edges_directed(node, Direction::Incoming).count();
        outgoing + incoming
    }

    /// Get high-coupling nodes (hotspots)
    /// Used by: overview tool
    #[must_use]
    pub fn find_hotspots(&self, limit: usize) -> Vec<(NodeIndex, usize)> {
        let mut scores: Vec<(NodeIndex, usize)> = self
            .graph
            .node_indices()
            .map(|n| (n, self.coupling_score(n)))
            .collect();

        scores.sort_by(|a, b| b.1.cmp(&a.1));
        scores.truncate(limit);
        scores
    }

    /// Get all dependents and dependencies for a symbol (for explain tool)
    #[must_use]
    pub fn get_symbol_relations(&self, node: NodeIndex) -> (SymbolRelations, SymbolRelations) {
        // Dependencies (what this symbol uses/calls)
        let dependencies: SymbolRelations = self
            .graph
            .edges(node)
            .map(|e| (e.target(), e.weight().relationship))
            .collect();

        // Dependents (what uses/calls this symbol)
        let dependents: SymbolRelations = self
            .graph
            .edges_directed(node, Direction::Incoming)
            .map(|e| (e.source(), e.weight().relationship))
            .collect();

        (dependencies, dependents)
    }

    /// Check if a symbol is part of public API (has "pub" visibility or used by tests)
    /// Used by: impact tool
    #[must_use]
    pub fn is_public_api(&self, node: NodeIndex) -> bool {
        // Check if used from test files
        let usages = self.get_all_usages(node);
        for (user, _rel) in &usages {
            if let Some(user_node) = self.get_node(*user) {
                if user_node.chunk_id.contains("test") || user_node.chunk_id.contains("_test.") {
                    return true;
                }
            }
        }
        // Also check if symbol name starts with pub (heuristic from chunk content)
        if let Some(node_data) = self.get_node(node) {
            if let Some(chunk) = &node_data.chunk {
                return chunk.content.trim_start().starts_with("pub ");
            }
        }
        false
    }

    /// Find test files related to a symbol
    /// Used by: impact, explain tools
    #[must_use]
    pub fn find_related_tests(&self, node: NodeIndex) -> Vec<NodeIndex> {
        let usages = self.get_transitive_usages(node, 3);
        usages
            .into_iter()
            .filter(|(n, _, _)| {
                self.get_node(*n)
                    .map(|nd| {
                        nd.chunk_id.contains("test")
                            || nd.chunk_id.contains("_test.")
                            || nd.symbol.name.starts_with("test_")
                    })
                    .unwrap_or(false)
            })
            .map(|(n, _, _)| n)
            .collect()
    }

    /// Get statistics about the graph
    /// Used by: overview tool
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (self.graph.node_count(), self.graph.edge_count())
    }
}
