use anyhow::{Context, Result};
use context_code_chunker::{ChunkMetadata, CodeChunk};
use context_vector_store::ChunkCorpus;
use rmcp::{model::CallToolRequestParam, service::ServiceExt, transport::TokioChildProcess};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

fn locate_context_finder_mcp_bin() -> Result<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_context-finder-mcp") {
        return Ok(PathBuf::from(path));
    }

    // Cargo doesn't always expose CARGO_BIN_EXE_* at runtime. Derive it from the test exe path:
    // `.../target/{debug|release}/deps/<test>` → `.../target/{debug|release}/context-finder-mcp`
    if let Ok(exe) = std::env::current_exe() {
        if let Some(target_profile_dir) = exe.parent().and_then(|p| p.parent()) {
            let candidate = target_profile_dir.join("context-finder-mcp");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(2)
        .context("failed to resolve repo root from CARGO_MANIFEST_DIR")?;
    for rel in [
        "target/debug/context-finder-mcp",
        "target/release/context-finder-mcp",
    ] {
        let candidate = repo_root.join(rel);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!("failed to locate context-finder-mcp binary")
}

#[tokio::test]
async fn mcp_exposes_core_tools_and_map_has_no_side_effects() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env_remove("CONTEXT_FINDER_MODEL_DIR");
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("RUST_LOG", "warn");
    cmd.env("CONTEXT_FINDER_DISABLE_DAEMON", "1");

    let transport = TokioChildProcess::new(cmd).context("spawn mcp server")?;
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .context("timeout starting MCP server")??;

    let tools = tokio::time::timeout(
        Duration::from_secs(10),
        service.list_tools(Default::default()),
    )
    .await
    .context("timeout listing tools")??;
    let tools_raw =
        serde_json::to_vec(&tools).context("serialize tools/list response for diagnostics")?;
    let tool_names: HashSet<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "map",
        "repo_onboarding_pack",
        "read_pack",
        "file_slice",
        "list_files",
        "grep_context",
        "batch",
        "doctor",
        "search",
        "context",
        "context_pack",
        "index",
        "text_search",
        "impact",
        "trace",
        "explain",
        "overview",
    ] {
        assert!(
            tool_names.contains(expected),
            "missing tool '{expected}' (available: {tool_names:?})"
        );
    }
    // Keep the tools/list payload reasonably sized so MCP clients don't choke on it.
    assert!(
        tools_raw.len() < 1_500_000,
        "tools/list payload is unexpectedly large ({} bytes)",
        tools_raw.len()
    );

    let tmp = tempfile::tempdir().context("tempdir")?;
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).context("mkdir src")?;
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .context("write main.rs")?;

    assert!(
        !root.join(".context-finder").exists(),
        "temp project unexpectedly has .context-finder before map"
    );

    let map_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "depth": 2,
        "limit": 20,
    });
    let map_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "map".into(),
            arguments: map_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling map")??;

    assert_ne!(map_result.is_error, Some(true), "map returned error");
    let map_text = map_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("map did not return text content")?;
    let map_json: Value = serde_json::from_str(map_text).context("map output is not valid JSON")?;

    assert!(
        map_json
            .get("total_files")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 1
    );
    assert!(map_json
        .get("directories")
        .and_then(Value::as_array)
        .is_some());

    assert!(
        !root.join(".context-finder").exists(),
        "map created .context-finder side effects"
    );

    let doctor_args = serde_json::json!({ "path": root.to_string_lossy() });
    let doctor_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "doctor".into(),
            arguments: doctor_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling doctor")??;

    assert_ne!(doctor_result.is_error, Some(true), "doctor returned error");
    let doctor_text = doctor_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("doctor did not return text content")?;
    let doctor_json: Value =
        serde_json::from_str(doctor_text).context("doctor output is not valid JSON")?;

    assert_eq!(
        doctor_json
            .get("env")
            .and_then(|v| v.get("profile"))
            .and_then(Value::as_str),
        Some("quality")
    );
    let project = doctor_json
        .get("project")
        .context("doctor did not return project info")?;
    let project_root = project
        .get("root")
        .and_then(Value::as_str)
        .context("doctor project.root missing")?;
    let corpus_path = project
        .get("corpus_path")
        .and_then(Value::as_str)
        .context("doctor project.corpus_path missing")?;

    let expected_root = root.canonicalize().context("canonicalize temp root")?;
    let reported_root = PathBuf::from(project_root)
        .canonicalize()
        .context("canonicalize reported root")?;
    assert_eq!(
        reported_root, expected_root,
        "doctor reported unexpected root (got: {project_root})"
    );

    let expected_corpus_path = expected_root.join(".context-finder").join("corpus.json");
    assert_eq!(
        PathBuf::from(corpus_path),
        expected_corpus_path,
        "doctor reported unexpected corpus_path"
    );
    assert_eq!(
        project
            .get("has_corpus")
            .and_then(Value::as_bool)
            .context("doctor project.has_corpus missing")?,
        expected_corpus_path.exists(),
        "doctor has_corpus must match corpus_path existence"
    );

    // Create a minimal corpus + index to validate drift diagnostics without requiring embedding models.
    std::fs::create_dir_all(
        root.join(".context-finder")
            .join("indexes")
            .join("bge-small"),
    )
    .context("mkdir indexes")?;

    let mut corpus = ChunkCorpus::new();
    corpus.set_file_chunks(
        "src/main.rs".to_string(),
        vec![CodeChunk::new(
            "src/main.rs".to_string(),
            1,
            1,
            "fn main() {}".to_string(),
            ChunkMetadata::default(),
        )],
    );
    corpus
        .save(root.join(".context-finder").join("corpus.json"))
        .await
        .context("save corpus")?;

    std::fs::write(
        root.join(".context-finder")
            .join("indexes")
            .join("bge-small")
            .join("index.json"),
        r#"{"schema_version":3,"dimension":384,"next_id":1,"id_map":{"0":"src/other.rs:1:1"},"vectors":{}}"#,
    )
    .context("write index.json")?;

    let doctor_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "doctor".into(),
            arguments: doctor_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling doctor (with corpus/index)")??;

    assert_ne!(
        doctor_result.is_error,
        Some(true),
        "doctor returned error (with corpus/index)"
    );
    let doctor_text = doctor_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("doctor did not return text content (with corpus/index)")?;
    let doctor_json: Value = serde_json::from_str(doctor_text)
        .context("doctor output is not valid JSON (with corpus/index)")?;

    assert_eq!(
        doctor_json
            .get("project")
            .and_then(|v| v.get("has_corpus"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        doctor_json
            .get("project")
            .and_then(|v| v.get("drift"))
            .and_then(Value::as_array)
            .map(|v| v.len()),
        Some(1)
    );
    assert_eq!(
        doctor_json
            .get("project")
            .and_then(|v| v.get("drift"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("missing_chunks"))
            .and_then(Value::as_u64),
        Some(1)
    );

    // Batch: one call → multiple tools, with a single bounded JSON output.
    let batch_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "max_chars": 20000,
        "items": [
            { "id": "map", "tool": "map", "input": { "depth": 2, "limit": 20 } },
            { "id": "doctor", "tool": "doctor", "input": {} }
        ]
    });

    let batch_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "batch".into(),
            arguments: batch_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling batch")??;

    assert_ne!(batch_result.is_error, Some(true), "batch returned error");
    let batch_text = batch_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("batch did not return text content")?;
    let batch_json: Value =
        serde_json::from_str(batch_text).context("batch output is not valid JSON")?;

    assert_eq!(
        batch_json.get("version").and_then(Value::as_u64),
        Some(1),
        "batch schema version mismatch"
    );
    let items = batch_json
        .get("items")
        .and_then(Value::as_array)
        .context("batch items is not an array")?;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get("id").and_then(Value::as_str), Some("map"));
    assert_eq!(items[0].get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(items[1].get("id").and_then(Value::as_str), Some("doctor"));
    assert_eq!(items[1].get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(
        batch_json
            .get("budget")
            .and_then(|v| v.get("truncated"))
            .and_then(Value::as_bool),
        Some(false)
    );

    service.cancel().await.context("shutdown mcp service")?;
    Ok(())
}

#[tokio::test]
async fn mcp_batch_truncates_when_budget_is_too_small() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env_remove("CONTEXT_FINDER_MODEL_DIR");
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("RUST_LOG", "warn");
    cmd.env("CONTEXT_FINDER_DISABLE_DAEMON", "1");

    let transport = TokioChildProcess::new(cmd).context("spawn mcp server")?;
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .context("timeout starting MCP server")??;

    let tmp = tempfile::tempdir().context("tempdir")?;
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).context("mkdir src")?;
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .context("write main.rs")?;

    let batch_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "max_chars": 200,
        "items": [
            { "id": "doctor", "tool": "doctor", "input": {} }
        ]
    });

    let batch_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "batch".into(),
            arguments: batch_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling batch (truncation)")??;

    assert_ne!(batch_result.is_error, Some(true), "batch returned error");
    let batch_text = batch_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("batch did not return text content")?;
    let batch_json: Value =
        serde_json::from_str(batch_text).context("batch output is not valid JSON")?;

    assert_eq!(
        batch_json
            .get("budget")
            .and_then(|v| v.get("truncated"))
            .and_then(Value::as_bool),
        Some(true)
    );

    let items = batch_json
        .get("items")
        .and_then(Value::as_array)
        .context("batch items is not an array")?;
    assert!(
        !items.is_empty(),
        "batch returned no items after truncation"
    );
    assert_eq!(
        items[0].get("status").and_then(Value::as_str),
        Some("error")
    );

    service.cancel().await.context("shutdown mcp service")?;
    Ok(())
}

#[tokio::test]
async fn mcp_file_slice_reads_bounded_lines_and_rejects_escape() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env_remove("CONTEXT_FINDER_MODEL_DIR");
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("RUST_LOG", "warn");
    cmd.env("CONTEXT_FINDER_DISABLE_DAEMON", "1");

    let transport = TokioChildProcess::new(cmd).context("spawn mcp server")?;
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .context("timeout starting MCP server")??;

    let tmp = tempfile::tempdir().context("tempdir")?;
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).context("mkdir src")?;
    std::fs::write(root.join("src").join("main.rs"), "line-1\nline-2\nline-3\n")
        .context("write main.rs")?;

    assert!(
        !root.join(".context-finder").exists(),
        "temp project unexpectedly has .context-finder before file_slice"
    );

    let slice_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "file": "src/main.rs",
        "start_line": 2,
        "max_lines": 2,
        "max_chars": 2000,
    });
    let slice_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "file_slice".into(),
            arguments: slice_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling file_slice")??;

    assert_ne!(
        slice_result.is_error,
        Some(true),
        "file_slice returned error"
    );
    let slice_text = slice_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("file_slice did not return text content")?;
    let slice_json: Value =
        serde_json::from_str(slice_text).context("file_slice output is not valid JSON")?;

    assert_eq!(
        slice_json.get("file").and_then(Value::as_str),
        Some("src/main.rs")
    );
    assert_eq!(
        slice_json.get("start_line").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(slice_json.get("end_line").and_then(Value::as_u64), Some(3));
    assert_eq!(
        slice_json.get("returned_lines").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        slice_json.get("truncated").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        slice_json.get("content").and_then(Value::as_str),
        Some("line-2\nline-3")
    );
    assert!(
        slice_json
            .get("file_size_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
    assert!(
        slice_json
            .get("content_sha256")
            .and_then(Value::as_str)
            .map(|s| s.len())
            .unwrap_or(0)
            == 64
    );

    assert!(
        !root.join(".context-finder").exists(),
        "file_slice created .context-finder side effects"
    );

    let outside_parent = root.parent().context("temp root has no parent")?;
    let outside = tempfile::NamedTempFile::new_in(outside_parent).context("outside temp file")?;
    std::fs::write(outside.path(), "nope").context("write outside file")?;
    let outside_name = outside
        .path()
        .file_name()
        .context("outside temp file has no file name")?
        .to_string_lossy()
        .into_owned();

    let escape_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "file": format!("../{outside_name}"),
        "start_line": 1,
        "max_lines": 10,
        "max_chars": 2000,
    });
    let escape_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "file_slice".into(),
            arguments: escape_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling file_slice (escape)")??;

    assert_eq!(
        escape_result.is_error,
        Some(true),
        "file_slice escape should error"
    );
    let escape_text = escape_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or_default();
    assert!(
        escape_text.contains("outside project root"),
        "unexpected escape error message: {escape_text}"
    );

    service.cancel().await.context("shutdown mcp service")?;
    Ok(())
}

#[tokio::test]
async fn mcp_list_files_lists_paths_and_is_bounded() -> Result<()> {
    let bin = locate_context_finder_mcp_bin()?;

    let mut cmd = Command::new(bin);
    cmd.env_remove("CONTEXT_FINDER_MODEL_DIR");
    cmd.env("CONTEXT_FINDER_PROFILE", "quality");
    cmd.env("RUST_LOG", "warn");
    cmd.env("CONTEXT_FINDER_DISABLE_DAEMON", "1");

    let transport = TokioChildProcess::new(cmd).context("spawn mcp server")?;
    let service = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
        .await
        .context("timeout starting MCP server")??;

    let tmp = tempfile::tempdir().context("tempdir")?;
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).context("mkdir src")?;
    std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n").context("write main.rs")?;
    std::fs::create_dir_all(root.join("docs")).context("mkdir docs")?;
    std::fs::write(root.join("docs").join("README.md"), "# Hello\n").context("write docs")?;
    std::fs::write(root.join("README.md"), "Root\n").context("write root readme")?;

    assert!(
        !root.join(".context-finder").exists(),
        "temp project unexpectedly has .context-finder before list_files"
    );

    let list_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "file_pattern": "src/*",
        "limit": 50,
        "max_chars": 20_000,
    });
    let list_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "list_files".into(),
            arguments: list_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling list_files")??;

    assert_ne!(
        list_result.is_error,
        Some(true),
        "list_files returned error"
    );
    let list_text = list_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("list_files did not return text content")?;
    let list_json: Value =
        serde_json::from_str(list_text).context("list_files output is not valid JSON")?;

    assert_eq!(
        list_json.get("source").and_then(Value::as_str),
        Some("filesystem")
    );
    let files = list_json
        .get("files")
        .and_then(Value::as_array)
        .context("list_files files is not an array")?;
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].as_str(),
        Some("src/main.rs"),
        "unexpected file path: {files:?}"
    );
    assert_eq!(
        list_json.get("truncated").and_then(Value::as_bool),
        Some(false)
    );

    for file in files {
        let Some(file) = file.as_str() else {
            continue;
        };
        assert!(
            !file.starts_with('/'),
            "list_files must return relative paths (got: {file})"
        );
        assert!(
            !file.contains(".."),
            "list_files must not return traversal paths (got: {file})"
        );
    }

    let limited_args = serde_json::json!({
        "path": root.to_string_lossy(),
        "limit": 1,
        "max_chars": 20_000,
    });
    let limited_result = tokio::time::timeout(
        Duration::from_secs(10),
        service.call_tool(CallToolRequestParam {
            name: "list_files".into(),
            arguments: limited_args.as_object().cloned(),
        }),
    )
    .await
    .context("timeout calling list_files (limited)")??;
    assert_ne!(
        limited_result.is_error,
        Some(true),
        "list_files (limited) returned error"
    );
    let limited_text = limited_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .context("list_files (limited) did not return text content")?;
    let limited_json: Value = serde_json::from_str(limited_text)
        .context("list_files (limited) output is not valid JSON")?;
    assert_eq!(
        limited_json.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        limited_json.get("truncation").and_then(Value::as_str),
        Some("limit")
    );

    assert!(
        !root.join(".context-finder").exists(),
        "list_files created .context-finder side effects"
    );

    service.cancel().await.context("shutdown mcp service")?;
    Ok(())
}
