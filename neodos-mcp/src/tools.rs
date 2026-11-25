use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use neodos_toolkit::database::Database;
use neodos_toolkit::analysis::{kernel, abi, consistency};
use neodos_toolkit::analysis::consistency::{ConsistencyIssue, Severity};

use crate::server::ToolSpec;

/// Resolve the actual NeoDOS checkout when the MCP root is its parent directory
/// (e.g. `~/rust-os` instead of `~/rust-os/neodos`).
fn neodos_root(root: &Path) -> PathBuf {
    if root.join("neodos-kernel").is_dir() {
        root.to_path_buf()
    } else if root.join("neodos").join("neodos-kernel").is_dir() {
        root.join("neodos")
    } else {
        root.to_path_buf()
    }
}

/// Locate a build artifact in the checkout, the parent workspace, or the
/// canonical cargo target directories.
fn resolve_artifact(root: &Path, name: &str) -> Option<PathBuf> {
    let mut candidates = vec![root.join(name)];
    if let Some(parent) = root.parent() {
        candidates.push(parent.join(name));
        candidates.push(parent.join("neodos").join(name));
    }
    candidates.push(root.join("neodos-kernel/target/x86_64-unknown-none/debug").join(name));
    candidates.push(root.join("neodos-bootloader/target/x86_64-unknown-uefi/debug").join(name));
    candidates.push(root.join("neodos-bootloader/target/x86_64-unknown-uefi/release").join(name));
    candidates.into_iter().find(|path| path.exists())
}

/// Collect files with the given extension, skipping `target` and hidden dirs.
fn collect_artifacts(dir: &Path, extension: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "target" || name.starts_with('.'))
                .unwrap_or(true);
            if !skip {
                collect_artifacts(&path, extension, depth - 1, out);
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

fn push_issue(
    issues: &mut Vec<ConsistencyIssue>,
    severity: Severity,
    category: &str,
    message: String,
) {
    issues.push(ConsistencyIssue {
        severity,
        category: category.to_string(),
        message,
    });
}

fn format_issue(issue: &ConsistencyIssue) -> String {
    let severity = match issue.severity {
        Severity::Error => "ERROR",
        Severity::Warning => "WARN",
        Severity::Info => "INFO",
    };
    format!("[{severity}] {}: {}", issue.category, issue.message)
}

fn check_artifacts(root: &Path, issues: &mut Vec<ConsistencyIssue>) {
    let required = [
        ("kernel.elf", Severity::Error),
        ("bootloader.efi", Severity::Error),
        ("disk_image.img", Severity::Warning),
    ];
    for (name, severity) in required {
        match resolve_artifact(root, name) {
            Some(path) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                push_issue(
                    issues,
                    Severity::Info,
                    "artifacts",
                    format!("{name} ({} bytes) at {}", size, path.display()),
                );
            }
            None => push_issue(
                issues,
                severity,
                "artifacts",
                format!("{name} not found (searched the checkout, parent workspace and target dirs)"),
            ),
        }
    }

    for (extension, label) in [("nxl", "NXL"), ("nem", "NEM")] {
        let mut files = Vec::new();
        collect_artifacts(root, extension, 4, &mut files);
        if files.is_empty() {
            push_issue(
                issues,
                Severity::Warning,
                "artifacts",
                format!("no .{extension} {label} artifacts found"),
            );
        } else {
            push_issue(
                issues,
                Severity::Info,
                "artifacts",
                format!("{} .{extension} {label} artifact(s)", files.len()),
            );
        }
    }
}

fn check_code(root: &Path, issues: &mut Vec<ConsistencyIssue>) {
    for rel in [
        "neodos-kernel/src/main.rs",
        "neodos-kernel/src/syscall/mod.rs",
        "neodos-kernel/src/object/types.rs",
        "neodos-kernel/src/fs/vfs/mod.rs",
        "libneodos/src/export.rs",
    ] {
        if root.join(rel).exists() {
            push_issue(issues, Severity::Info, "code", format!("{rel} present"));
        } else {
            push_issue(issues, Severity::Error, "code", format!("{rel} missing"));
        }
    }
}

fn check_docs(root: &Path, issues: &mut Vec<ConsistencyIssue>) {
    for rel in [
        "docs/README.md",
        "docs/kernel/syscalls.md",
        "docs/kernel/objects.md",
        "docs/architecture/source-of-truth.md",
    ] {
        if root.join(rel).exists() {
            push_issue(issues, Severity::Info, "docs", format!("{rel} present"));
        } else {
            push_issue(issues, Severity::Warning, "docs", format!("{rel} missing"));
        }
    }
}

pub struct McpTools {
    root: PathBuf,
    db: Arc<Database>,
}

impl McpTools {
    pub fn new(root: PathBuf, db: Arc<Database>) -> Self {
        Self { root, db }
    }

    fn make_tool(
        name: &'static str,
        desc: &'static str,
        schema: serde_json::Value,
        handler: impl Fn(&HashMap<String, serde_json::Value>) -> String + 'static + Send + Sync,
    ) -> ToolSpec {
        ToolSpec {
            name,
            description: desc,
            input_schema: schema,
            handler: Box::new(handler),
        }
    }
}

impl McpTools {
    pub fn kernel_index(&self) -> ToolSpec {
        let root = self.root.clone();
        Self::make_tool(
            "kernel_index",
            "List all kernel source files with line counts, grouped by subsystem",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "format": {
                        "type": "string",
                        "enum": ["tree", "summary"],
                        "default": "tree"
                    }
                }
            }),
            move |params| {
                let _format = params.get("format").and_then(|v| v.as_str()).unwrap_or("tree");
                let info = kernel::get_kernel_info(&root);
                let mut out = format!("NeoDOS v{}\n", info.version);
                out.push_str("=== Kernel Architecture ===\n");
                for (name, desc) in &info.subsystems {
                    out.push_str(&format!("  {name}: {desc}\n"));
                }
                out
            },
        )
    }

    pub fn search_symbol(&self) -> ToolSpec {
        Self::make_tool(
            "search_symbol",
            "Search for function, struct, trait, or const definitions in kernel source",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "integer", "default": 30 }
                },
                "required": ["query"]
            }),
            {
                let db = self.db.clone();
                move |params| {
                    let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let max = params.get("max_results").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
                    let results = kernel::search_symbols(&db, query, max);
                    if results.is_empty() {
                        return format!("No results for '{query}'");
                    }
                    results.join("\n")
                }
            },
        )
    }

    pub fn get_kernel_architecture(&self) -> ToolSpec {
        Self::make_tool(
            "get_kernel_architecture",
            "Return kernel memory layout, boot phase sequence, subsystem boundaries",
            serde_json::json!({"type": "object", "properties": {}}),
            {
                let root = self.root.clone();
                move |_| {
                    let info = kernel::get_kernel_info(&root);
                    let mut out = format!("NeoDOS v{}\n", info.version);
                    out.push_str("\n=== Boot Phases ===\n");
                    for phase in &info.boot_phases {
                        out.push_str(&format!("  {phase}\n"));
                    }
                    out.push_str("\n=== Subsystems ===\n");
                    for (name, desc) in &info.subsystems {
                        out.push_str(&format!("  {name:<15} {desc}\n"));
                    }
                    out
                }
            },
        )
    }

    pub fn get_build_errors(&self) -> ToolSpec {
        Self::make_tool(
            "get_build_errors",
            "Check for build issues: missing artifacts, ABI mismatches",
            serde_json::json!({"type": "object", "properties": {}}),
            {
                let root = self.root.clone();
                move |_| {
                    let neodos = neodos_root(&root);
                    let mut issues = Vec::new();
                    check_artifacts(&neodos, &mut issues);
                    issues.iter().map(format_issue).collect::<Vec<_>>().join("\n")
                }
            },
        )
    }

    pub fn list_loaded_modules(&self) -> ToolSpec {
        Self::make_tool(
            "list_loaded_modules",
            "List NEM drivers and NXLs found in build artifacts",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "enum": ["all", "nem", "dll"],
                        "default": "all"
                    }
                }
            }),
            {
                let root = self.root.clone();
                move |params| {
                    let category = params.get("category").and_then(|v| v.as_str()).unwrap_or("all");
                    let neodos = neodos_root(&root);
                    let mut out = String::new();

                    if category == "all" || category == "nem" {
                        let mut files = Vec::new();
                        collect_artifacts(&neodos, "nem", 4, &mut files);
                        files.sort();
                        out.push_str(&format!("NEM Drivers ({}):\n", files.len()));
                        for file in &files {
                            let rel = file.strip_prefix(&neodos).unwrap_or(file);
                            out.push_str(&format!("  {}\n", rel.display()));
                        }
                    }

                    if category == "all" || category == "dll" {
                        let mut files = Vec::new();
                        collect_artifacts(&neodos, "nxl", 3, &mut files);
                        files.sort();
                        out.push_str(&format!("\nNXL Libraries ({}):\n", files.len()));
                        for file in &files {
                            let rel = file.strip_prefix(&neodos).unwrap_or(file);
                            out.push_str(&format!("  {}\n", rel.display()));
                        }
                    }

                    if out.is_empty() {
                        out.push_str("No modules found.\n");
                    }
                    out
                }
            },
        )
    }

    pub fn check_abi_compatibility(&self) -> ToolSpec {
        Self::make_tool(
            "check_abi_compatibility",
            "Check ABI compatibility between a NEM driver and the kernel",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "abi_min": { "type": "integer" },
                    "abi_target": { "type": "integer" },
                    "abi_max": { "type": "integer" }
                },
                "required": ["abi_min", "abi_target", "abi_max"]
            }),
            move |params| {
                let min = params.get("abi_min").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                let target = params.get("abi_target").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                let max = params.get("abi_max").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

                let driver_abi = abi::AbiVersion { min, target, max };
                let result = abi::check_nem_abi(&driver_abi);
                format!(
                    "Driver ABI: min={min}, target={target}, max={max}\nKernel ABI: {}\n{}",
                    abi::KERNEL_ABI_VERSION,
                    abi::format_compatibility(&result)
                )
            },
        )
    }

    pub fn check_consistency(&self) -> ToolSpec {
        let root = self.root.clone();
        Self::make_tool(
            "check_consistency",
            "Validate architectural consistency: code, docs, artifacts, invariants",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "targets": {
                        "type": "string",
                        "enum": ["all", "code", "docs", "artifacts", "invariants"],
                        "default": "all"
                    }
                }
            }),
            move |params| {
                let target = params
                    .get("targets")
                    .and_then(|value| value.as_str())
                    .unwrap_or("all");
                let neodos = neodos_root(&root);
                let mut issues = Vec::new();

                if target == "all" || target == "invariants" {
                    issues.extend(consistency::check_invariants());
                }
                if target == "all" || target == "artifacts" {
                    check_artifacts(&neodos, &mut issues);
                }
                if target == "all" || target == "code" {
                    check_code(&neodos, &mut issues);
                }
                if target == "all" || target == "docs" {
                    check_docs(&neodos, &mut issues);
                }

                let mut out = format!("=== Consistency Check ({target}) ===\n");
                for issue in &issues {
                    out.push_str(&format_issue(issue));
                    out.push('\n');
                }
                if target == "all" || target == "invariants" {
                    out.push_str(&format!("\nInvariants ({}):\n", consistency::INVARIANTS.len()));
                    for (id, desc) in consistency::INVARIANTS {
                        out.push_str(&format!("  {id}: {desc}\n"));
                    }
                }
                out
            },
        )
    }

    pub fn boot_phases(&self) -> ToolSpec {
        Self::make_tool(
            "boot_phases",
            "Describe kernel boot phases and initialization sequence",
            serde_json::json!({"type": "object", "properties": {}}),
            {
                let root = self.root.clone();
                move |_| {
                    let info = kernel::get_kernel_info(&root);
                    let mut out = String::from("Boot Phases:\n");
                    for phase in &info.boot_phases {
                        out.push_str(&format!("  {phase}\n"));
                    }
                    out
                }
            },
        )
    }

    pub fn memory_layout(&self) -> ToolSpec {
        Self::make_tool(
            "memory_layout",
            "Show kernel memory layout: regions, allocators, page tables",
            serde_json::json!({"type": "object", "properties": {}}),
            move |_| {
                String::from(
                    "Kernel Memory Layout:\n\
                     0x00000000 - 0x00001000: Null page (captures null derefs)\n\
                     0x00001000 - 0x00100000: Bootloader + BootInfo\n\
                     0x00100000 - 0x00400000: Kernel code + data\n\
                     0x00400000 - 0x01000000: Kernel heap (buddy + slab)\n\
                     0x01000000 - 0x04000000: Page tables + MMIO\n\
                     0x04000000 - 0x10000000: NEM isolation regions (16×16MB)\n\
                     0x10000000 - 0x20000000: NXL shared libraries\n\
                     0x20000000 - 0x40000000: User address space (Ring 3)\n\
                    \nAllocators:\n\
                      Buddy: power-of-2, bitmap, max 4 GB\n\
                      Slab: per-size caches (32B, 64B, 128B, 256B, 512B, 1KB, 2KB, 4KB)\n\
                      Linked-list: fallback for large allocations"
                )
            },
        )
    }

    pub fn security_info(&self) -> ToolSpec {
        Self::make_tool(
            "security_info",
            "Show security subsystem: SIDs, tokens, ACL structure",
            serde_json::json!({"type": "object", "properties": {}}),
            move |_| {
                String::from(
                    "Security Reference Monitor:\n\
                     - SID: Security Identifier (S-1-5-21-{rid})\n\
                     - Token: integrity_level + creation_time + group SIDs\n\
                     - ACL: Discretionary + System ACL\n\
                     - SAM: built-in Administrator (500) + Guest (501)\n\
                     - SeAccessCheck: object permission validation\n\
                     - Integrity levels: Untrusted(0), Low(1), Medium(2), High(3), System(4)"
                )
            },
        )
    }

    pub fn scheduler_info(&self) -> ToolSpec {
        Self::make_tool(
            "scheduler_info",
            "Show scheduler state: processes, threads, run queues, priorities",
            serde_json::json!({"type": "object", "properties": {}}),
            move |_| {
                String::from(
                    "Scheduler:\n\
                     - Priority levels: 0 (idle) .. 31 (realtime)\n\
                     - Per-CPU run queues with work stealing\n\
                     - SMP: IPI, TLB shootdown\n\
                     - Timeslice: ~30ms default\n\
                     - Aging: priority boost after wait\n\
                     - KWait: blocking engine (events, timers, I/O)\n\
                     - DPC: Deferred Procedure Calls\n\
                     - APC: Asynchronous Procedure Calls"
                )
            },
        )
    }

    pub fn ipc_info(&self) -> ToolSpec {
        Self::make_tool(
            "ipc_info",
            "Show IPC subsystem: pipes, IRP pool, work queue, event bus",
            serde_json::json!({"type": "object", "properties": {}}),
            move |_| {
                String::from(
                    "IPC Subsystem:\n\
                     - Pipes: ring buffer, 4KB×16 slots, zero-copy (planned)\n\
                     - IRP: I/O Request Packets, pool-allocated\n\
                     - Work queue: kernel worker thread pool\n\
                     - Event Bus: centralized, priority-ordered, typed events\n\
                     - Handles: Ob handle table, per-process"
                )
            },
        )
    }
}

impl McpTools {
    pub fn all_tools(&self) -> Vec<ToolSpec> {
        vec![
            self.kernel_index(),
            self.search_symbol(),
            self.get_kernel_architecture(),
            self.get_build_errors(),
            self.list_loaded_modules(),
            self.check_abi_compatibility(),
            self.check_consistency(),
            self.boot_phases(),
            self.memory_layout(),
            self.security_info(),
            self.scheduler_info(),
            self.ipc_info(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fixture(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "neodos-mcp-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let neodos = base.join("neodos");
        std::fs::create_dir_all(&neodos).unwrap();
        for rel in [
            "neodos-kernel/src/main.rs",
            "neodos-kernel/src/syscall/mod.rs",
            "neodos-kernel/src/object/types.rs",
            "neodos-kernel/src/fs/vfs/mod.rs",
            "libneodos/src/export.rs",
            "docs/README.md",
            "docs/kernel/syscalls.md",
            "docs/kernel/objects.md",
            "docs/architecture/source-of-truth.md",
        ] {
            let path = neodos.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fixture").unwrap();
        }
        for rel in ["kernel.elf", "bootloader.efi", "disk_image.img", "libfoo.nxl"] {
            std::fs::write(neodos.join(rel), "artifact").unwrap();
        }
        std::fs::create_dir_all(neodos.join("drivers/pci")).unwrap();
        std::fs::write(neodos.join("drivers/pci/pci.nem"), "artifact").unwrap();
        base
    }

    fn call(tool: ToolSpec, params: &[(&str, &str)]) -> String {
        let mut map = HashMap::new();
        for (key, value) in params {
            map.insert((*key).to_string(), serde_json::Value::String((*value).to_string()));
        }
        (tool.handler)(&map)
    }

    #[test]
    fn resolves_checkout_under_parent_root() {
        let base = fixture("root");
        let root = neodos_root(&base);
        assert_eq!(root, base.join("neodos"));
        assert!(resolve_artifact(&root, "kernel.elf").is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_errors_finds_artifacts() {
        let base = fixture("build");
        let tools = McpTools::new(base.clone(), Arc::new(Database::new()));
        let out = call(tools.get_build_errors(), &[]);
        assert!(out.contains("kernel.elf ("), "{out}");
        assert!(out.contains("bootloader.efi ("), "{out}");
        assert!(!out.contains("kernel.elf not found"), "{out}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn modules_list_built_artifacts() {
        let base = fixture("modules");
        let tools = McpTools::new(base.clone(), Arc::new(Database::new()));
        let out = call(tools.list_loaded_modules(), &[("category", "all")]);
        assert!(out.contains("pci.nem"), "{out}");
        assert!(out.contains("libfoo.nxl"), "{out}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn consistency_honours_targets() {
        let base = fixture("consistency");
        let tools = McpTools::new(base.clone(), Arc::new(Database::new()));

        let artifacts = call(tools.check_consistency(), &[("targets", "artifacts")]);
        assert!(artifacts.contains("(artifacts)"), "{artifacts}");
        assert!(!artifacts.contains("Invariants ("), "{artifacts}");

        let docs = call(tools.check_consistency(), &[("targets", "docs")]);
        assert!(docs.contains("(docs)"), "{docs}");
        assert!(!docs.contains("kernel.elf ("), "{docs}");

        let all = call(tools.check_consistency(), &[("targets", "all")]);
        assert!(all.contains("Invariants ("), "{all}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
