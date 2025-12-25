//! Context Finder MCP tool surface.
//!
//! This module is intentionally split into submodules to keep schemas, dispatch, and per-tool
//! implementations reviewable and evolvable.

mod batch;
mod cursor;
mod dispatch;
mod file_slice;
mod grep_context;
mod list_files;
mod map;
mod paths;
mod repo_onboarding_pack;
mod schemas;
mod util;

pub use dispatch::ContextFinderService;
