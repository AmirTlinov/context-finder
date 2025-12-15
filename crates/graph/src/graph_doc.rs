use crate::types::{RelationshipType, SymbolType};
use crate::{ContextAssembler, GraphNode};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

pub const GRAPH_DOC_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct GraphDoc {
    pub node_id: String,
    pub chunk_id: String,
    pub doc: String,
    pub doc_hash: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphDocConfig {
    pub max_neighbors_per_relation: usize,
}

impl Default for GraphDocConfig {
    fn default() -> Self {
        Self {
            max_neighbors_per_relation: 12,
        }
    }
}

/// Build deterministic "graph documents" (graph-as-text) for embedding/search.
///
/// Design goals:
/// - stable ordering (diff-friendly),
/// - bounded size (neighbor caps),
/// - useful tokens for conceptual/architecture queries.
#[must_use]
pub fn build_graph_docs(assembler: &ContextAssembler, config: GraphDocConfig) -> Vec<GraphDoc> {
    let graph = assembler.graph();
    let mut docs = Vec::new();

    for (idx, node) in graph.nodes() {
        let (doc, doc_hash) = render_graph_doc(graph, idx, node, config);
        docs.push(GraphDoc {
            node_id: node_key(node),
            chunk_id: node.chunk_id.clone(),
            doc,
            doc_hash,
        });
    }

    docs.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    docs
}

fn render_graph_doc(
    graph: &crate::CodeGraph,
    idx: NodeIndex,
    node: &GraphNode,
    config: GraphDocConfig,
) -> (String, u64) {
    let symbol = &node.symbol;
    let mut out = String::new();

    let display_name = symbol
        .qualified_name
        .as_deref()
        .unwrap_or(symbol.name.as_str());

    out.push_str("kind: graph_node\n");
    out.push_str(&format!("node_id: {}\n", node_key(node)));
    out.push_str(&format!("chunk_id: {}\n", node.chunk_id));
    out.push_str(&format!("symbol: {display_name}\n"));
    out.push_str(&format!(
        "symbol_type: {}\n",
        symbol_type_name(&symbol.symbol_type)
    ));
    out.push_str(&format!("file: {}\n", symbol.file_path));
    out.push_str(&format!(
        "lines: {}-{}\n",
        symbol.start_line, symbol.end_line
    ));
    out.push_str(&format!("graph_doc_version: {GRAPH_DOC_VERSION}\n"));

    for direction in [Direction::Outgoing, Direction::Incoming] {
        let dir_name = match direction {
            Direction::Outgoing => "out",
            Direction::Incoming => "in",
        };

        for rel in rel_order() {
            let neighbors = collect_neighbors(graph, idx, direction, rel, config);
            out.push_str(&format!(
                "{dir_name}.{}({}):\n",
                rel_name(rel),
                neighbors.len()
            ));
            for neighbor in neighbors {
                out.push_str("- ");
                out.push_str(&neighbor);
                out.push('\n');
            }
        }
    }

    let doc_hash = fnv1a64(out.as_bytes());
    (out, doc_hash)
}

fn collect_neighbors(
    graph: &crate::CodeGraph,
    idx: NodeIndex,
    direction: Direction,
    rel: RelationshipType,
    config: GraphDocConfig,
) -> Vec<String> {
    let mut neighbors: Vec<String> = graph
        .graph
        .edges_directed(idx, direction)
        .filter(|edge| edge.weight().relationship == rel)
        .filter_map(|edge| {
            let other = match direction {
                Direction::Outgoing => edge.target(),
                Direction::Incoming => edge.source(),
            };
            graph.graph.node_weight(other).map(|node| {
                let sym = &node.symbol;
                let label = sym.qualified_name.as_deref().unwrap_or(sym.name.as_str());
                format!("{label} | {}", sym.file_path)
            })
        })
        .collect();

    neighbors.sort();
    neighbors.dedup();
    neighbors.truncate(config.max_neighbors_per_relation);
    neighbors
}

fn node_key(node: &GraphNode) -> String {
    let display = node
        .symbol
        .qualified_name
        .as_deref()
        .unwrap_or(node.symbol.name.as_str());
    format!("{}#{}", node.chunk_id, display)
}

fn rel_order() -> [RelationshipType; 6] {
    [
        RelationshipType::Calls,
        RelationshipType::Uses,
        RelationshipType::Imports,
        RelationshipType::Contains,
        RelationshipType::Extends,
        RelationshipType::TestedBy,
    ]
}

fn symbol_type_name(kind: &SymbolType) -> &'static str {
    match kind {
        SymbolType::Function => "function",
        SymbolType::Method => "method",
        SymbolType::Class => "class",
        SymbolType::Struct => "struct",
        SymbolType::Enum => "enum",
        SymbolType::Interface => "interface",
        SymbolType::Variable => "variable",
        SymbolType::Constant => "constant",
        SymbolType::Module => "module",
    }
}

fn rel_name(rel: RelationshipType) -> &'static str {
    match rel {
        RelationshipType::Calls => "calls",
        RelationshipType::Uses => "uses",
        RelationshipType::Imports => "imports",
        RelationshipType::Contains => "contains",
        RelationshipType::Extends => "extends",
        RelationshipType::TestedBy => "tested_by",
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 14695981039346656037;
    const PRIME: u64 = 1099511628211;
    let mut hash = OFFSET;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CodeGraph, GraphEdge, GraphNode, RelationshipType, Symbol};

    fn mk_symbol(name: &str, file: &str, start: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified_name: Some(format!("demo::{name}")),
            file_path: file.to_string(),
            start_line: start,
            end_line: start + 1,
            symbol_type: SymbolType::Function,
        }
    }

    #[test]
    fn graph_docs_are_deterministic() {
        let mut graph = CodeGraph::new();

        let a = GraphNode {
            symbol: mk_symbol("a", "a.rs", 1),
            chunk_id: "a.rs:1:2".to_string(),
            chunk: None,
        };
        let b = GraphNode {
            symbol: mk_symbol("b", "b.rs", 10),
            chunk_id: "b.rs:10:11".to_string(),
            chunk: None,
        };

        let ia = graph.add_node(a);
        let ib = graph.add_node(b);
        graph.add_edge(
            ia,
            ib,
            GraphEdge {
                relationship: RelationshipType::Calls,
                weight: 1.0,
            },
        );

        let assembler = ContextAssembler::new(graph);
        let first = build_graph_docs(&assembler, GraphDocConfig::default());
        let second = build_graph_docs(&assembler, GraphDocConfig::default());

        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.node_id, b.node_id);
            assert_eq!(a.doc_hash, b.doc_hash);
            assert_eq!(a.doc, b.doc);
        }
    }
}
