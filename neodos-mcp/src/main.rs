mod server;
mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use neodos_toolkit::database::Database;
use neodos_toolkit::config::NeodosConfig;
use neodos_toolkit::indexer::Indexer;

use crate::server::McpServer;
use crate::tools::*;

fn find_neodos_root() -> PathBuf {
    if let Ok(val) = std::env::var("NEODOS_ROOT") {
        return PathBuf::from(val);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut path = Some(cwd.as_path());
    while let Some(p) = path {
        if p.join("AGENTS.md").exists() {
            if let Ok(content) = std::fs::read_to_string(p.join("AGENTS.md")) {
                if content.contains("NeoDOS") {
                    return p.to_path_buf();
                }
            }
        }
        if p.join("neodos-kernel").is_dir() {
            return p.to_path_buf();
        }
        path = p.parent();
    }
    std::env::current_dir().unwrap_or_default()
}

fn get_neodos_version(root: &std::path::Path) -> String {
    let agents = root.join("AGENTS.md");
    if let Ok(content) = std::fs::read_to_string(&agents) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(ver) = trimmed.strip_prefix("**Version:** v") {
                return ver.trim().to_string();
            }
        }
    }
    "dev".to_string()
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("neodos_mcp=info")
    )
    .format_timestamp_millis()
    .init();

    let root_dir = find_neodos_root();
    unsafe { std::env::set_var("NEODOS_ROOT", &root_dir); }
    let version = get_neodos_version(&root_dir);

    log::info!(
        "NeoDOS MCP v{} starting (neodos v{}, root: {})",
        env!("CARGO_PKG_VERSION"), version, root_dir.display()
    );

    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("NeoDOS MCP Server v{}", env!("CARGO_PKG_VERSION"));
        println!("Environment: NEODOS_ROOT (default: auto-detect)");
        println!("Runs MCP protocol over stdio (JSON-RPC 2.0)");
        return;
    }

    let config = Arc::new(NeodosConfig::detection());
    let db = Arc::new(Database::new());

    let indexer = Indexer::new(db.clone(), config.clone());
    let files = indexer.discover_files();
    let count = indexer.index_workspace(&files);
    *db.all_files.write() = files;
    log::info!("indexed {} symbols for MCP", count);

    let tools = McpTools::new(root_dir, db.clone());

    let mut server = McpServer::new("neodos-mcp", &version);
    server.register_all_tools(&tools);

    log::info!("MCP server ready with {} tools", server.tool_count());

    server.run_stdio();
}
