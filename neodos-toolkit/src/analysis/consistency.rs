/// Architectural consistency checks and invariants.

#[derive(Debug, Clone)]
pub struct ConsistencyIssue {
    pub severity: Severity,
    pub category: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

pub const INVARIANTS: &[(&str, &str)] = &[
    ("INV-1", "No Ring 0 shell commands. All interactive commands in userbin/ as .NXE"),
    ("INV-2", "Syscall dispatch via SSDT (O(1)), not match. RAX is the index."),
    ("INV-3", "Every resource is an Ob object. No raw kernel resource outside Ob."),
    ("INV-4", "Drivers run in isolated NEM sandbox. No direct kernel memory access."),
    ("INV-5", "New syscalls (RAX ≥ 60) MUST be sys_ob_* operating on Ob objects."),
    ("INV-6", "HAL is the only module with inline assembly. All arch code behind HAL."),
    ("INV-7", "No circular dependencies between kernel subsystems."),
    ("INV-8", "100% Rust. No C or C++ in the kernel."),
    ("INV-9", "SSDT ABI is frozen within a major kernel version (semver)."),
    ("INV-10", "NEM ABI is versioned. Drivers declare min/target/max versions."),
];

pub fn check_invariants() -> Vec<ConsistencyIssue> {
    vec![
        ConsistencyIssue {
            severity: Severity::Info,
            category: "invariants".into(),
            message: format!("{} architectural invariants defined", INVARIANTS.len()),
        }
    ]
}
