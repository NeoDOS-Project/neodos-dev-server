use std::path::Path;

use crate::database::Database;

/// Kernel architecture information
pub struct KernelInfo {
    pub version: String,
    pub boot_phases: Vec<&'static str>,
    pub subsystems: Vec<(&'static str, &'static str)>,
}

pub fn get_kernel_info(root: &Path) -> KernelInfo {
    let version = read_version_from_agents(root);
    KernelInfo {
        version,
        boot_phases: vec![
            "Phase 0: Early boot (GDT, IDT, CPU init)",
            "Phase 1: Memory (page tables, allocators)",
            "Phase 2: Kernel services (scheduler, HAL)",
            "Phase 3: Driver loading (NEM boot drivers)",
            "Phase 4: Userland init (spawn neoinit)",
        ],
        subsystems: vec![
            ("scheduler", "SMP priority scheduler with work stealing"),
            ("memory", "Buddy allocator, slab, demand paging"),
            ("drivers", "NEM v3 driver framework with isolation"),
            ("fs", "NeoFS v2 + VFS with mount support"),
            ("net", "TCP/IP stack, ARP, DHCP, e1000"),
            ("registry", "Cell-based Registry hive persistence"),
            ("security", "SID, Token, ACL, SAM, SeAccessCheck"),
            ("object", "Object Manager (Ob) — 16 types, 7 syscalls"),
            ("hal", "x86_64 HAL: GDT, IDT, IOAPIC, MSI-X"),
            ("services", "Service Manager for system services"),
        ],
    }
}

pub fn search_symbols(db: &Database, query: &str, max: usize) -> Vec<String> {
    db.find_by_name(query)
        .iter()
        .take(max)
        .map(|s| format!("{}:{}:{}", s.file.display(), s.range.start.line + 1, s.name))
        .collect()
}

fn read_version_from_agents(root: &Path) -> String {
    let agents = root.join("AGENTS.md");
    if let Ok(content) = std::fs::read_to_string(&agents) {
        for line in content.lines() {
            if let Some(ver) = line.strip_prefix("**Version:** v") {
                return ver.trim().to_string();
            }
        }
    }
    "dev".to_string()
}
