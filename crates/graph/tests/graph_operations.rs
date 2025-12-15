//! Tests for CodeGraph operations

use context_graph::{CodeGraph, GraphEdge, GraphNode, RelationshipType, Symbol, SymbolType};

fn make_symbol(name: &str, file: &str, start: usize, end: usize, sym_type: SymbolType) -> Symbol {
    Symbol {
        name: name.to_string(),
        qualified_name: Some(format!("{}::{}", file.replace(".rs", ""), name)),
        file_path: file.to_string(),
        start_line: start,
        end_line: end,
        symbol_type: sym_type,
    }
}

fn make_node(name: &str, chunk_id: &str, file: &str) -> GraphNode {
    GraphNode {
        symbol: make_symbol(name, file, 1, 10, SymbolType::Function),
        chunk_id: chunk_id.to_string(),
        chunk: None,
    }
}

fn make_edge(rel: RelationshipType) -> GraphEdge {
    GraphEdge {
        relationship: rel,
        weight: 1.0,
    }
}

#[test]
fn test_add_node_and_find() {
    let mut graph = CodeGraph::new();

    let node = make_node("foo", "chunk_1", "src/lib.rs");
    let idx = graph.add_node(node);

    assert!(graph.find_node("foo").is_some());
    assert_eq!(graph.find_node("foo"), Some(idx));
    assert!(graph.find_node("bar").is_none());
}

#[test]
fn test_node_count() {
    let mut graph = CodeGraph::new();

    assert_eq!(graph.node_count(), 0);

    graph.add_node(make_node("foo", "chunk_1", "src/lib.rs"));
    assert_eq!(graph.node_count(), 1);

    graph.add_node(make_node("bar", "chunk_2", "src/lib.rs"));
    assert_eq!(graph.node_count(), 2);
}

#[test]
fn test_edge_count() {
    let mut graph = CodeGraph::new();

    let n1 = graph.add_node(make_node("foo", "chunk_1", "src/lib.rs"));
    let n2 = graph.add_node(make_node("bar", "chunk_2", "src/lib.rs"));

    assert_eq!(graph.edge_count(), 0);

    graph.add_edge(n1, n2, make_edge(RelationshipType::Calls));
    assert_eq!(graph.edge_count(), 1);
}

#[test]
fn test_get_callees() {
    let mut graph = CodeGraph::new();

    let n_main = graph.add_node(make_node("main", "chunk_1", "src/main.rs"));
    let n_helper = graph.add_node(make_node("helper", "chunk_2", "src/lib.rs"));
    let n_util = graph.add_node(make_node("util", "chunk_3", "src/util.rs"));

    // main calls helper and util
    graph.add_edge(n_main, n_helper, make_edge(RelationshipType::Calls));
    graph.add_edge(n_main, n_util, make_edge(RelationshipType::Calls));

    let callees = graph.get_callees(n_main);
    assert_eq!(callees.len(), 2);
    assert!(callees.contains(&n_helper));
    assert!(callees.contains(&n_util));

    // helper has no callees
    assert!(graph.get_callees(n_helper).is_empty());
}

#[test]
fn test_get_callers() {
    let mut graph = CodeGraph::new();

    let n_main = graph.add_node(make_node("main", "chunk_1", "src/main.rs"));
    let n_run = graph.add_node(make_node("run", "chunk_2", "src/lib.rs"));
    let n_helper = graph.add_node(make_node("helper", "chunk_3", "src/lib.rs"));

    // main -> helper, run -> helper
    graph.add_edge(n_main, n_helper, make_edge(RelationshipType::Calls));
    graph.add_edge(n_run, n_helper, make_edge(RelationshipType::Calls));

    let callers = graph.get_callers(n_helper);
    assert_eq!(callers.len(), 2);
    assert!(callers.contains(&n_main));
    assert!(callers.contains(&n_run));

    // main has no callers
    assert!(graph.get_callers(n_main).is_empty());
}

#[test]
fn test_get_dependencies() {
    let mut graph = CodeGraph::new();

    let n_service = graph.add_node(make_node("Service", "chunk_1", "src/service.rs"));
    let n_config = graph.add_node(make_node("Config", "chunk_2", "src/config.rs"));
    let n_db = graph.add_node(make_node("Database", "chunk_3", "src/db.rs"));

    // Service uses Config and Database
    graph.add_edge(n_service, n_config, make_edge(RelationshipType::Uses));
    graph.add_edge(n_service, n_db, make_edge(RelationshipType::Uses));

    let deps = graph.get_dependencies(n_service);
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&n_config));
    assert!(deps.contains(&n_db));
}

#[test]
fn test_get_nodes_by_relationship() {
    let mut graph = CodeGraph::new();

    let n_a = graph.add_node(make_node("A", "chunk_1", "a.rs"));
    let n_b = graph.add_node(make_node("B", "chunk_2", "b.rs"));
    let n_c = graph.add_node(make_node("C", "chunk_3", "c.rs"));

    // A calls B, A uses C
    graph.add_edge(n_a, n_b, make_edge(RelationshipType::Calls));
    graph.add_edge(n_a, n_c, make_edge(RelationshipType::Uses));

    let calls = graph.get_nodes_by_relationship(n_a, RelationshipType::Calls);
    assert_eq!(calls.len(), 1);
    assert!(calls.contains(&n_b));

    let uses = graph.get_nodes_by_relationship(n_a, RelationshipType::Uses);
    assert_eq!(uses.len(), 1);
    assert!(uses.contains(&n_c));
}

#[test]
fn test_get_related_nodes_depth_1() {
    let mut graph = CodeGraph::new();

    let n_a = graph.add_node(make_node("A", "chunk_1", "a.rs"));
    let n_b = graph.add_node(make_node("B", "chunk_2", "b.rs"));
    let n_c = graph.add_node(make_node("C", "chunk_3", "c.rs"));

    // A -> B -> C
    graph.add_edge(n_a, n_b, make_edge(RelationshipType::Calls));
    graph.add_edge(n_b, n_c, make_edge(RelationshipType::Calls));

    // Depth 1: only B
    let related = graph.get_related_nodes(n_a, 1);
    assert_eq!(related.len(), 1);
    assert!(related.iter().any(|(idx, _, _)| *idx == n_b));
}

#[test]
fn test_get_related_nodes_depth_2() {
    let mut graph = CodeGraph::new();

    let n_a = graph.add_node(make_node("A", "chunk_1", "a.rs"));
    let n_b = graph.add_node(make_node("B", "chunk_2", "b.rs"));
    let n_c = graph.add_node(make_node("C", "chunk_3", "c.rs"));

    // A -> B -> C
    graph.add_edge(n_a, n_b, make_edge(RelationshipType::Calls));
    graph.add_edge(n_b, n_c, make_edge(RelationshipType::Calls));

    // Depth 2: B and C
    let related = graph.get_related_nodes(n_a, 2);
    assert_eq!(related.len(), 2);
    assert!(related.iter().any(|(idx, _, _)| *idx == n_b));
    assert!(related.iter().any(|(idx, _, _)| *idx == n_c));
}

#[test]
fn test_find_path() {
    let mut graph = CodeGraph::new();

    let n_a = graph.add_node(make_node("A", "chunk_1", "a.rs"));
    let n_b = graph.add_node(make_node("B", "chunk_2", "b.rs"));
    let n_c = graph.add_node(make_node("C", "chunk_3", "c.rs"));

    // A -> B -> C
    graph.add_edge(n_a, n_b, make_edge(RelationshipType::Calls));
    graph.add_edge(n_b, n_c, make_edge(RelationshipType::Calls));

    // Path A -> C exists
    let path = graph.find_path(n_a, n_c);
    assert!(path.is_some());

    // No path C -> A (directed graph)
    let no_path = graph.find_path(n_c, n_a);
    assert!(no_path.is_none());
}

#[test]
fn test_find_nodes_by_chunk() {
    let mut graph = CodeGraph::new();

    // Two nodes in same chunk
    let n1 = graph.add_node(make_node("foo", "chunk_1", "lib.rs"));
    let n2 = graph.add_node(make_node("bar", "chunk_1", "lib.rs"));
    let _n3 = graph.add_node(make_node("baz", "chunk_2", "lib.rs"));

    let nodes = graph.find_nodes_by_chunk("chunk_1");
    assert_eq!(nodes.len(), 2);
    assert!(nodes.contains(&n1));
    assert!(nodes.contains(&n2));
}

#[test]
fn test_get_context_for_symbol() {
    let mut graph = CodeGraph::new();

    let n_main = graph.add_node(make_node("main", "chunk_main", "main.rs"));
    let n_helper = graph.add_node(make_node("helper", "chunk_helper", "helper.rs"));
    let n_util = graph.add_node(make_node("util", "chunk_util", "util.rs"));

    // main -> helper -> util
    graph.add_edge(n_main, n_helper, make_edge(RelationshipType::Calls));
    graph.add_edge(n_helper, n_util, make_edge(RelationshipType::Calls));

    // Get context for main with depth 2
    let chunks = graph.get_context_for_symbol("main", 2).unwrap();
    assert!(chunks.contains(&"chunk_main".to_string()));
    assert!(chunks.contains(&"chunk_helper".to_string()));
    assert!(chunks.contains(&"chunk_util".to_string()));
}

#[test]
fn test_get_context_for_unknown_symbol() {
    let graph = CodeGraph::new();

    let result = graph.get_context_for_symbol("unknown", 2);
    assert!(result.is_err());
}

#[test]
fn test_relationship_types() {
    let mut graph = CodeGraph::new();

    let n_class = graph.add_node(make_node("MyClass", "chunk_1", "class.rs"));
    let n_method = graph.add_node(make_node("my_method", "chunk_1", "class.rs"));
    let n_test = graph.add_node(make_node("test_my_method", "chunk_2", "tests.rs"));
    let n_dep = graph.add_node(make_node("Dependency", "chunk_3", "dep.rs"));

    // MyClass contains my_method
    graph.add_edge(n_class, n_method, make_edge(RelationshipType::Contains));

    // test tests my_method
    graph.add_edge(n_test, n_method, make_edge(RelationshipType::TestedBy));

    // MyClass uses Dependency
    graph.add_edge(n_class, n_dep, make_edge(RelationshipType::Uses));

    assert_eq!(
        graph.get_nodes_by_relationship(n_class, RelationshipType::Contains),
        vec![n_method]
    );
    assert_eq!(
        graph.get_nodes_by_relationship(n_test, RelationshipType::TestedBy),
        vec![n_method]
    );
    assert_eq!(
        graph.get_nodes_by_relationship(n_class, RelationshipType::Uses),
        vec![n_dep]
    );
}
