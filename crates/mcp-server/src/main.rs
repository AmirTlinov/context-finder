//! Context Finder MCP Server
//!
//! Provides semantic code search capabilities to AI agents via MCP protocol.
//!
//! ## Tools
//!
//! - `repo_onboarding_pack` - Map + key docs + next_actions (best first call)
//! - `read_pack` - One-call file/grep/query/onboarding with cursor continuation
//! - `context_pack` - Bounded semantic pack (primary + related halo)
//! - `batch` - Multiple tool calls under one max_chars budget with $ref
//! - `file_slice` - Bounded file window (root-locked)
//! - `grep_context` - Regex matches with before/after context hunks
//! - `list_files` - Bounded file enumeration (glob/substring filter)
//! - `text_search` - Bounded text search (corpus or FS fallback)
//! - `search` - Semantic search using natural language
//! - `context` - Search with automatic graph-based context (calls, dependencies)
//! - `impact` - Find symbol usages and transitive impact
//! - `trace` - Call chain between two symbols
//! - `explain` - Symbol details, deps, dependents, docs
//! - `overview` - Architecture snapshot (layers, entry points)
//! - `map` - Project structure overview (directories, files, top symbols)
//! - `index` - Index a project directory for semantic search
//! - `doctor` - Diagnose model/GPU/index configuration
//!
//! ## Usage
//!
//! Add to your MCP client configuration:
//! ```json
//! {
//!   "mcpServers": {
//!     "context-finder": {
//!       "command": "context-finder-mcp"
//!     }
//!   }
//! }
//! ```

use anyhow::Result;
use rmcp::ServiceExt;
use std::env;

mod daemon;
mod runtime_env;
mod stdio_hybrid;
mod tools;

use stdio_hybrid::stdio_hybrid_server;
use tools::catalog;
use tools::ContextFinderService;

fn print_help() {
    println!("Context Finder MCP server");
    println!();
    println!("Usage: context-finder-mcp [--print-tools|--version|--help]");
    println!();
    println!("Flags:");
    println!("  --print-tools  Print tool inventory as JSON and exit");
    println!("  --version      Print version and exit");
    println!("  --help         Print this help and exit");
}

fn handle_cli_args() -> Option<i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return None;
    }

    if args.len() == 1 {
        match args[0].as_str() {
            "--print-tools" => {
                let payload = catalog::tool_inventory_json(env!("CARGO_PKG_VERSION"));
                println!("{}", payload);
                return Some(0);
            }
            "--version" | "-V" => {
                println!("context-finder-mcp {}", env!("CARGO_PKG_VERSION"));
                return Some(0);
            }
            "--help" | "-h" => {
                print_help();
                return Some(0);
            }
            _ => {}
        }
    }

    eprintln!("Unknown arguments: {}", args.join(" "));
    print_help();
    Some(2)
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(exit_code) = handle_cli_args() {
        std::process::exit(exit_code);
    }

    // Configure logging to stderr only (stdout is for MCP protocol)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Stderr)
        .filter_module("ort", log::LevelFilter::Off) // Silence ONNX Runtime
        .init();

    let bootstrap = runtime_env::bootstrap_best_effort();
    for warning in &bootstrap.warnings {
        log::warn!("{warning}");
    }

    log::info!("Starting Context Finder MCP server");

    // Create and start the MCP server
    let service = ContextFinderService::new();
    let server = service.serve(stdio_hybrid_server()).await?;

    // Wait for shutdown
    server.waiting().await?;

    log::info!("Context Finder MCP server stopped");
    Ok(())
}
