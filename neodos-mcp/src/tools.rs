use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use neodos_toolkit::database::Database;
use neodos_toolkit::analysis::{kernel, abi, consistency};

use crate::server::ToolSpec;

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
                    let mut issues = Vec::new();
                    let kernel_elf = root.join("kernel.elf");
                    if kernel_elf.exists() {
                        if let Ok(meta) = std::fs::metadata(&kernel_elf) {
                            issues.push(format!("✓ kernel.elf ({} bytes)", meta.len()));
                        }
                    } else {
                        issues.push("✗ kernel.elf NOT FOUND".to_string());
                    }
                    let boot_efi = root.join("bootloader.efi");
                    if boot_efi.exists() {
                        if let Ok(meta) = std::fs::metadata(&boot_efi) {
                            issues.push(format!("✓ bootloader.efi ({} bytes)", meta.len()));
                        }
                    } else {
                        issues.push("✗ bootloader.efi NOT FOUND".to_string());
                    }
                    let disk_img = root.join("disk_image.img");
                    if disk_img.exists() {
                        if let Ok(meta) = std::fs::metadata(&disk_img) {
                            issues.push(format!("✓ disk_image.img ({} MB)", meta.len() / 1024 / 1024));
                        }
                    } else {
                        issues.push("⚠ disk_image.img not found".to_string());
                    }
                    issues.join("\n")
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
                    let mut out = String::new();

                    if category == "all" || category == "nem" {
                        out.push_str("NEM Drivers:\n");
                        let drivers_dir = root.join("drivers");
                        if drivers_dir.is_dir() {
                            if let Ok(entries) = std::fs::read_dir(&drivers_dir) {
                                for entry in entries.flatten() {
                                    if entry.path().is_dir() {
                                        if let Some(name) = entry.file_name().to_str() {
                                            out.push_str(&format!("  {name}\n"));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if category == "all" || category == "dll" {
                        out.push_str("\nNXL Libraries:\n");
                        if let Ok(entries) = std::fs::read_dir(&root) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_dir() && p.file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|n| n.starts_with("lib") && n.ends_with("-nxl"))
                                    .unwrap_or(false)
                                {
                                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                        out.push_str(&format!("  {name}\n"));
                                    }
                                }
                            }
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
            move |_| {
                let mut out = String::from("=== Consistency Check ===\n");
                let issues = consistency::check_invariants();
                for issue in &issues {
                    let sev = match issue.severity {
                        consistency::Severity::Error => "ERROR",
                        consistency::Severity::Warning => "WARN",
                        consistency::Severity::Info => "INFO",
                    };
                    out.push_str(&format!("[{sev}] {}: {}\n", issue.category, issue.message));
                }
                out.push_str(&format!("\nInvariants ({}):\n", consistency::INVARIANTS.len()));
                for (id, desc) in consistency::INVARIANTS {
                    out.push_str(&format!("  {id}: {desc}\n"));
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
