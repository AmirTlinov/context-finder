use crate::error::Result;
use crate::types::{CodeGraph, RelationshipType};
use context_code_chunker::CodeChunk;
use std::cmp::Ordering;

/// Smart context assembler for AI agents
///
/// Automatically gathers related code chunks based on graph relationships
pub struct ContextAssembler {
    graph: CodeGraph,
}

/// Context assembly strategy
#[derive(Debug, Clone, Copy)]
pub enum AssemblyStrategy {
    /// Include direct dependencies only (depth=1)
    Direct,

    /// Include dependencies and their dependencies (depth=2)
    Extended,

    /// Include full call chain (depth=3)
    Deep,

    /// Custom depth
    Custom(usize),
}

/// Assembled context for AI agent
#[derive(Debug, Clone)]
pub struct AssembledContext {
    /// Primary chunk (the one requested)
    pub primary_chunk: CodeChunk,

    /// Related chunks with relationship info
    pub related_chunks: Vec<RelatedChunk>,

    /// Total context size (for token estimation)
    pub total_lines: usize,
}

#[derive(Debug, Clone)]
pub struct RelatedChunk {
    pub chunk: CodeChunk,
    pub relationship: Vec<RelationshipType>,
    pub distance: usize,
    pub relevance_score: f32,
}

fn relationship_rank(rel: RelationshipType) -> u8 {
    match rel {
        RelationshipType::Calls => 0,
        RelationshipType::Uses => 1,
        RelationshipType::Contains => 2,
        RelationshipType::Extends => 3,
        RelationshipType::Imports => 4,
        RelationshipType::TestedBy => 5,
    }
}

fn compare_relationship_paths(a: &[RelationshipType], b: &[RelationshipType]) -> Ordering {
    for (&left, &right) in a.iter().zip(b.iter()) {
        let left = relationship_rank(left);
        let right = relationship_rank(right);
        match left.cmp(&right) {
            Ordering::Equal => {}
            non_eq => return non_eq,
        }
    }
    a.len().cmp(&b.len())
}

impl ContextAssembler {
    #[must_use]
    pub const fn new(graph: CodeGraph) -> Self {
        Self { graph }
    }

    /// Assemble context for a symbol
    pub fn assemble_for_symbol(
        &self,
        symbol_name: &str,
        strategy: AssemblyStrategy,
    ) -> Result<AssembledContext> {
        let max_depth = match strategy {
            AssemblyStrategy::Direct => 1,
            AssemblyStrategy::Extended => 2,
            AssemblyStrategy::Deep => 3,
            AssemblyStrategy::Custom(d) => d,
        };

        // Find primary node
        let node = self
            .graph
            .find_node(symbol_name)
            .ok_or_else(|| crate::error::GraphError::NodeNotFound(symbol_name.to_string()))?;

        // Get primary chunk
        let primary_node = self
            .graph
            .get_node(node)
            .ok_or_else(|| crate::error::GraphError::NodeNotFound(symbol_name.to_string()))?;

        let primary_chunk = primary_node.chunk.clone().ok_or_else(|| {
            crate::error::GraphError::BuildError("Missing chunk data".to_string())
        })?;

        // Get related nodes
        let related_nodes = self.graph.get_related_nodes(node, max_depth);

        // Build related chunks with scores
        let mut related_chunks = Vec::new();
        for (rel_node, distance, path) in related_nodes {
            if let Some(node_data) = self.graph.get_node(rel_node) {
                if let Some(chunk) = &node_data.chunk {
                    let relevance = Self::calculate_relevance(distance, &path);
                    related_chunks.push(RelatedChunk {
                        chunk: chunk.clone(),
                        relationship: path,
                        distance,
                        relevance_score: relevance,
                    });
                }
            }
        }

        // Sort by relevance
        related_chunks.sort_by(|a, b| {
            b.relevance_score
                .total_cmp(&a.relevance_score)
                .then_with(|| a.distance.cmp(&b.distance))
                .then_with(|| a.chunk.file_path.cmp(&b.chunk.file_path))
                .then_with(|| a.chunk.start_line.cmp(&b.chunk.start_line))
                .then_with(|| a.chunk.end_line.cmp(&b.chunk.end_line))
                .then_with(|| compare_relationship_paths(&a.relationship, &b.relationship))
        });

        // Calculate total lines
        let total_lines = primary_chunk.line_count()
            + related_chunks
                .iter()
                .map(|rc| rc.chunk.line_count())
                .sum::<usize>();

        Ok(AssembledContext {
            primary_chunk,
            related_chunks,
            total_lines,
        })
    }

    /// Assemble context for a chunk ID
    pub fn assemble_for_chunk(
        &self,
        chunk_id: &str,
        strategy: AssemblyStrategy,
    ) -> Result<AssembledContext> {
        // Find nodes for this chunk
        let nodes = self.graph.find_nodes_by_chunk(chunk_id);

        if nodes.is_empty() {
            return Err(crate::error::GraphError::NodeNotFound(chunk_id.to_string()));
        }

        // Use first node's symbol name
        let node = self
            .graph
            .get_node(nodes[0])
            .ok_or_else(|| crate::error::GraphError::NodeNotFound(chunk_id.to_string()))?;

        self.assemble_for_symbol(&node.symbol.name, strategy)
    }

    /// Calculate relevance score based on distance and relationship path
    #[allow(clippy::cast_precision_loss)]
    fn calculate_relevance(distance: usize, path: &[RelationshipType]) -> f32 {
        // Base score decreases with distance
        let distance_score = 1.0 / (distance as f32 + 1.0);

        // Relationship type weights
        let relationship_score: f32 = path
            .iter()
            .map(|rel| match rel {
                RelationshipType::Calls => 1.0,    // Direct call = highest relevance
                RelationshipType::Uses => 0.8,     // Type usage = high relevance
                RelationshipType::Contains => 0.7, // Parent-child = medium-high
                RelationshipType::Imports => 0.5,  // Import = medium relevance
                RelationshipType::Extends => 0.6,  // Inheritance = medium relevance
                RelationshipType::TestedBy => 0.4, // Test = lower relevance
            })
            .sum::<f32>()
            / path.len().max(1) as f32;

        distance_score * relationship_score
    }

    /// Get statistics about assembled context
    #[must_use]
    pub fn get_stats(&self) -> ContextStats {
        ContextStats {
            total_nodes: self.graph.node_count(),
            total_edges: self.graph.edge_count(),
        }
    }

    /// Batch assemble contexts for multiple symbols
    #[must_use]
    pub fn assemble_batch(
        &self,
        symbol_names: &[&str],
        strategy: AssemblyStrategy,
    ) -> Vec<Result<AssembledContext>> {
        symbol_names
            .iter()
            .map(|name| self.assemble_for_symbol(name, strategy))
            .collect()
    }

    #[must_use]
    pub fn graph(&self) -> &CodeGraph {
        &self.graph
    }
}

#[derive(Debug, Clone)]
pub struct ContextStats {
    pub total_nodes: usize,
    pub total_edges: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GraphEdge, GraphNode, Symbol, SymbolType};
    use context_code_chunker::ChunkMetadata;

    #[test]
    fn test_calculate_relevance() {
        let _assembler = ContextAssembler::new(CodeGraph::new());

        // Direct call (distance=1)
        let score1 = ContextAssembler::calculate_relevance(1, &[RelationshipType::Calls]);
        assert!(score1 > 0.4);

        // Distant relationship (distance=3)
        let score2 = ContextAssembler::calculate_relevance(
            3,
            &[
                RelationshipType::Calls,
                RelationshipType::Uses,
                RelationshipType::Calls,
            ],
        );
        assert!(score2 < score1);
    }

    #[test]
    fn related_chunks_are_sorted_deterministically() {
        let mut graph = CodeGraph::new();

        let mk_chunk = |path: &str, start: usize, end: usize| {
            CodeChunk::new(
                path.to_string(),
                start,
                end,
                format!("// {path}:{start}:{end}"),
                ChunkMetadata::default(),
            )
        };

        let mk_node = |name: &str, path: &str, start: usize, end: usize| GraphNode {
            symbol: Symbol {
                name: name.to_string(),
                qualified_name: None,
                file_path: path.to_string(),
                start_line: start,
                end_line: end,
                symbol_type: SymbolType::Function,
            },
            chunk_id: format!("{path}:{start}:{end}"),
            chunk: Some(mk_chunk(path, start, end)),
        };

        let primary = graph.add_node(mk_node("primary", "main.rs", 1, 10));
        let rel_a = graph.add_node(mk_node("a", "a.rs", 5, 6));
        let rel_b = graph.add_node(mk_node("b", "b.rs", 1, 2));
        let rel_c = graph.add_node(mk_node("c", "b.rs", 3, 4));

        for rel in [rel_a, rel_b, rel_c] {
            graph.add_edge(
                primary,
                rel,
                GraphEdge {
                    relationship: RelationshipType::Calls,
                    weight: 1.0,
                },
            );
        }

        let assembler = ContextAssembler::new(graph);
        let assembled = assembler
            .assemble_for_symbol("primary", AssemblyStrategy::Direct)
            .unwrap();

        let ordered: Vec<(String, usize, usize)> = assembled
            .related_chunks
            .iter()
            .map(|rc| {
                (
                    rc.chunk.file_path.clone(),
                    rc.chunk.start_line,
                    rc.chunk.end_line,
                )
            })
            .collect();

        assert_eq!(
            ordered,
            vec![
                ("a.rs".to_string(), 5, 6),
                ("b.rs".to_string(), 1, 2),
                ("b.rs".to_string(), 3, 4),
            ]
        );
    }
}
