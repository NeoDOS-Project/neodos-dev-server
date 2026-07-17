use std::path::PathBuf;
use std::sync::Arc;

use lsp_types::{Position, Range, SymbolKind};
use walkdir::WalkDir;

use crate::config::NeodosConfig;
use crate::database::{
    self, Database, FileIndex, NeodosItem, NeodosItemKind, Symbol,
};

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub symbols: Vec<database::Symbol>,
    pub references: Vec<database::Reference>,
    pub neodos_items: Vec<database::NeodosItem>,
}

struct SyscallNumVariant {
    name: String,
    number: u64,
}

pub struct Indexer {
    db: Arc<Database>,
    config: Arc<NeodosConfig>,
}

impl Indexer {
    pub fn new(db: Arc<Database>, config: Arc<NeodosConfig>) -> Self {
        Self { db, config }
    }

    pub fn discover_files(&self) -> Vec<PathBuf> {
        let exclude = &self.config.workspace.exclude_patterns;
        let max = self.config.workspace.max_files;
        let mut files = Vec::new();

        for root in self.config.workspace.roots.read().iter() {
            if !root.exists() {
                log::warn!("workspace root does not exist: {}", root.display());
                continue;
            }
            for entry in WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    let name = e.file_name().to_str().unwrap_or("");
                    if name.starts_with('.') && e.depth() == 1 {
                        return false;
                    }
                    let path = e.path().to_string_lossy();
                    !exclude.iter().any(|pat| {
                        if pat.ends_with("/**") {
                            path.contains(&pat[..pat.len() - 3])
                        } else {
                            path.contains(pat)
                        }
                    })
                })
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file()
                    && entry.path().extension().map(|e| e == "rs").unwrap_or(false)
                {
                    files.push(entry.path().to_path_buf());
                    if files.len() >= max {
                        log::warn!("hit max_files limit ({})", max);
                        return files;
                    }
                }
            }
        }

        log::info!("discovered {} .rs files in workspace", files.len());
        files
    }

    pub fn index_workspace(&self, files: &[PathBuf]) -> usize {
        log::info!("indexing {} files ({} threads)...", files.len(), self.parallelism());

        use rayon::prelude::*;
        let results: Vec<_> = files
            .par_iter()
            .with_max_len(8)
            .map(|path| {
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(e) => {
                        log::warn!("cannot read {}: {e}", path.display());
                        return None;
                    }
                };
                let parsed = Self::parse_file(path, &content);
                Some((path.clone(), content, parsed))
            })
            .collect();

        let mut count = 0;
        for result in results.into_iter().flatten() {
            let (path, _content, parsed) = result;
            let fi = FileIndex {
                file: path,
                symbols: parsed.symbols,
                references: parsed.references,
                neodos_items: parsed.neodos_items,
            };
            let sc = fi.symbols.len();
            self.db.replace_file_index(fi);
            count += sc;
        }

        *self.db.all_files.write() = files.to_vec();
        log::info!("indexing complete: {} symbols in {} files", count, files.len());
        count
    }

    fn parallelism(&self) -> usize {
        let c = self.config.indexing.threads;
        if c == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        } else {
            c
        }
    }

    pub fn parse_file(path: &std::path::Path, content: &str) -> ParsedFile {
        let mut symbols: Vec<database::Symbol> = Vec::new();
        let references: Vec<database::Reference> = Vec::new();
        let mut neodos_items: Vec<database::NeodosItem> = Vec::new();

        let lines: Vec<&str> = content.lines().collect();
        let mut current_comment: Option<String> = None;
        let mut pending_attrs: Vec<String> = Vec::new();
        let mut in_impl_block: Option<String> = None;
        let mut impl_brace: u32 = 0;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let line_trimmed_start = line.len() - line.trim_start().len();
            let col = line_trimmed_start as u32;

            if trimmed.starts_with("///") || trimmed.starts_with("//!") {
                let doc = trimmed.trim_start_matches("///").trim_start_matches("//!").trim();
                let prev = current_comment.take().unwrap_or_default();
                current_comment = Some(if prev.is_empty() { doc.to_string() } else { format!("{prev}\n{doc}") });
                continue;
            }
            if trimmed.starts_with("/*") && trimmed.contains("*/") {
                continue;
            }

            if trimmed.starts_with("#[") && trimmed.ends_with(']') {
                pending_attrs.push(trimmed[2..trimmed.len() - 1].to_string());
                continue;
            }
            if trimmed.starts_with("#[") {
                pending_attrs.push(trimmed.strip_prefix("#[").unwrap().to_string());
                continue;
            }
            if trimmed.ends_with(']') && !pending_attrs.is_empty() {
                let last = pending_attrs.last_mut().unwrap();
                last.push_str(trimmed.trim_end_matches(']'));
                continue;
            }

            let i = i as u32;
            let pos_start = Position { line: i, character: col };
            let pos_end = Position { line: i, character: (line.len()) as u32 };

            if let Some(mod_name) = Self::parse_mod_decl(trimmed) {
                let sym = Self::make_sym(
                    &mod_name, SymbolKind::MODULE, path, pos_start, pos_end,
                    &mut current_comment, &pending_attrs,
                );
                symbols.push(sym);
                pending_attrs.clear();
                continue;
            }

            if in_impl_block.is_some() {
                impl_brace += trimmed.matches('{').count() as u32;
                impl_brace = impl_brace.saturating_sub(trimmed.matches('}').count() as u32);

                if trimmed.starts_with("pub fn") || trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub unsafe fn") || trimmed.starts_with("unsafe fn")
                {
                    if let Some(name) = Self::extract_name(trimmed, "fn") {
                        let mut sym = Self::make_sym(
                            &name, SymbolKind::METHOD, path, pos_start, pos_end,
                            &mut current_comment, &pending_attrs,
                        );
                        sym.signature = Some(Self::extract_signature(trimmed, &lines, i as usize));

                        if name == "read" || name == "write" || name == "open" || name == "close" {
                            neodos_items.push(NeodosItem {
                                kind: NeodosItemKind::FileSystemImpl,
                                name: name.clone(),
                                detail: format!("impl {}::{}",
                                    in_impl_block.as_ref().unwrap(), name),
                            });
                        }

                        symbols.push(sym);
                    }
                    pending_attrs.clear();
                    continue;
                }

                if impl_brace == 0 {
                    in_impl_block = None;
                }
            }

            if trimmed.starts_with("pub fn") || trimmed.starts_with("fn ")
                || trimmed.starts_with("pub(crate) fn") || trimmed.starts_with("pub(super) fn")
                || trimmed.starts_with("pub unsafe fn") || trimmed.starts_with("unsafe fn")
                || trimmed.starts_with("pub async fn") || trimmed.starts_with("async fn")
            {
                let is_pub = trimmed.contains("pub ");
                if let Some(name) = Self::extract_name(trimmed, "fn") {
                    let kind = SymbolKind::FUNCTION;
                    let mut sym = Self::make_sym(
                        &name, kind, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    );
                    sym.visibility = Some(if is_pub { "pub".into() } else { "private".into() });
                    sym.signature = Some(Self::extract_signature(trimmed, &lines, i as usize));
                    sym.is_test = trimmed.contains("#[test]") || pending_attrs.iter().any(|a| a == "test");

                    if let Some(num) = Self::detect_syscall_handler(&name, &pending_attrs) {
                        sym.neodos_kind = Some(database::NeodosKind::Syscall(num));
                        sym.syscall_number = Some(num);
                    }
                    if name.starts_with("PHASE_") || name.starts_with("phase_") {
                        sym.neodos_kind = Some(database::NeodosKind::BootPhase);
                    }

                    let ndk = sym.neodos_kind;
                    symbols.push(sym);

                    match ndk {
                        Some(database::NeodosKind::Syscall(num)) => {
                            neodos_items.push(NeodosItem {
                                kind: NeodosItemKind::SyscallHandler,
                                name: name.clone(),
                                detail: format!("syscall #{num}: {name}"),
                            });
                        }
                        Some(database::NeodosKind::BootPhase) => {
                            neodos_items.push(NeodosItem {
                                kind: NeodosItemKind::BootPhaseFn,
                                name: name.clone(),
                                detail: format!("Boot phase: {name}"),
                            });
                        }
                        _ => {}
                    }
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("pub struct") || trimmed.starts_with("struct ") {
                if let Some(name) = Self::extract_name(trimmed, "struct") {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::STRUCT, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("pub enum") || trimmed.starts_with("enum ") {
                if let Some(name) = Self::extract_name(trimmed, "enum") {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::ENUM, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("pub trait") || trimmed.starts_with("trait ") {
                if let Some(name) = Self::extract_name(trimmed, "trait") {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::INTERFACE, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("pub type") || trimmed.starts_with("type ") {
                if let Some(name) = Self::extract_name(trimmed, "type") {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::TYPE_PARAMETER, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if (trimmed.starts_with("pub const") || trimmed.starts_with("const ")) && !trimmed.contains(" fn ") {
                if let Some(name) = Self::extract_name(trimmed, "const") {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::CONSTANT, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));

                    if name.starts_with("CAP_") {
                        let val = trimmed.split('=').nth(1).unwrap_or("?").trim().trim_end_matches(';').to_string();
                        neodos_items.push(NeodosItem {
                            kind: NeodosItemKind::CapabilityFlag,
                            name: name.clone(),
                            detail: format!("Capability: {name} = {val}"),
                        });
                    }
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("pub static") || trimmed.starts_with("static ") {
                if let Some(name) = Self::extract_name(trimmed, "static") {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::CONSTANT, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("macro_rules!") {
                if let Some(name) = Self::extract_macro_name(trimmed) {
                    symbols.push(Self::make_sym(
                        &name, SymbolKind::FUNCTION, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if trimmed.starts_with("impl ") {
                if let Some(target) = Self::extract_impl_target(trimmed) {
                    in_impl_block = Some(target.clone());
                    impl_brace = 1;
                    symbols.push(Self::make_sym(
                        &target, SymbolKind::OBJECT, path, pos_start, pos_end,
                        &mut current_comment, &pending_attrs,
                    ));
                }
                pending_attrs.clear();
                continue;
            }

            if let Some(var) = Self::parse_syscallnum_variant(trimmed) {
                neodos_items.push(NeodosItem {
                    kind: NeodosItemKind::SyscallHandler,
                    name: format!("SyscallNum::{} ({})", var.name, var.number),
                    detail: format!("syscall #{}: {}", var.number, var.name),
                });
                continue;
            }

            if trimmed.contains("CommandEntry {") || trimmed.contains("CommandEntry::new") {
                if let Some(cmd) = Self::parse_command_entry(trimmed, &lines, i as usize) {
                    neodos_items.push(NeodosItem {
                        kind: NeodosItemKind::ShellCommand,
                        name: cmd.name,
                        detail: cmd.detail,
                    });
                }
                continue;
            }

            if Self::is_driver_state_enum(trimmed) {
                neodos_items.push(NeodosItem {
                    kind: NeodosItemKind::DriverState,
                    name: trimmed.trim().trim_end_matches(',').to_string(),
                    detail: "DriverState variant".into(),
                });
                continue;
            }

            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                pending_attrs.clear();
            }
        }

        ParsedFile { symbols, references, neodos_items }
    }

    fn parse_mod_decl(trimmed: &str) -> Option<String> {
        if let Some(rest) = trimmed.strip_prefix("pub mod ").or_else(|| trimmed.strip_prefix("mod ")) {
            let name = rest.split([';', '{', ' ']).next().unwrap_or("").trim();
            if !name.is_empty() && !name.contains("//") {
                return Some(name.to_string());
            }
        }
        None
    }

    fn extract_name(trimmed: &str, keyword: &str) -> Option<String> {
        let kw_pos = trimmed.find(keyword)?;
        let after_kw = &trimmed[kw_pos + keyword.len()..];
        let name = after_kw
            .split(|c: char| c.is_whitespace() || c == '<' || c == '(' || c == ';' || c == '{' || c == '!')
            .find(|s| !s.is_empty())
            .unwrap_or("")
            .trim();
        if name.is_empty() || name.contains("//") {
            return None;
        }
        if matches!(name, "mut" | "ref" | "self" | "&" | "unsafe" | "async") {
            return None;
        }
        Some(name.to_string())
    }

    fn extract_macro_name(trimmed: &str) -> Option<String> {
        let rest = trimmed.strip_prefix("macro_rules!")?.trim();
        let name = rest.split('{').next().unwrap_or("").trim().trim_matches('!');
        if name.is_empty() { None } else { Some(name.to_string()) }
    }

    fn extract_impl_target(trimmed: &str) -> Option<String> {
        let rest = trimmed.strip_prefix("impl ")?;
        let target = rest.split("for ")
            .last()
            .unwrap_or(rest)
            .split('<').next().unwrap_or("")
            .split('{').next().unwrap_or("")
            .split("where").next().unwrap_or("")
            .trim();
        if target.is_empty() || target == " " { None } else { Some(target.to_string()) }
    }

    fn extract_signature(line: &str, lines: &[&str], line_idx: usize) -> String {
        let mut sig = line.to_string();
        if !sig.contains('{') {
            for l in lines.iter().take(lines.len().min(line_idx + 10)).skip(line_idx + 1) {
                sig.push_str(l);
                if l.contains('{') { break; }
            }
        }
        if let Some(pos) = sig.find('{') {
            sig.truncate(pos);
        }
        sig.trim().to_string()
    }

    fn make_sym(
        name: &str, kind: SymbolKind, path: &std::path::Path,
        start: Position, end: Position,
        doc: &mut Option<String>, attrs: &[String],
    ) -> Symbol {
        let doc_comment = doc.take();
        let is_deprecated = attrs.iter().any(|a| a.contains("deprecated"));
        Symbol {
            id: database::fresh_symbol_id(),
            name: name.to_string(),
            kind,
            neodos_kind: None,
            file: path.to_path_buf(),
            range: Range { start, end },
            selection_range: Range { start, end },
            parent: None,
            children: Vec::new(),
            documentation: doc_comment,
            detail: None,
            signature: None,
            visibility: None,
            attributes: attrs.to_vec(),
            is_deprecated,
            is_test: false,
            syscall_number: None,
            capabilities: None,
        }
    }

    fn detect_syscall_handler(name: &str, attrs: &[String]) -> Option<u64> {
        if let Some(rest) = name.strip_prefix("sys_") {
            if let Ok(num) = rest.parse::<u64>() {
                return Some(num);
            }
            Some(match rest {
                "exit" => 0, "write" => 1, "yield" => 2, "getpid" => 3,
                "read" => 4, "pipe" => 5, "dup2" => 6, "spawn" => 7,
                "readdir" => 8, "waitpid" => 9, "open" => 10,
                "readfile" => 11, "writefile" => 12, "close" => 13,
                "chdir" => 16, "getcwd" => 17, "brk" => 18, "mmap" => 19,
                "munmap" => 20, "loadlib" => 21, "thread_create" => 22,
                "thread_join" => 23, "getcpuinfo" => 24, "mkdir" => 25,
                "unlink" => 26, "rmdir" => 27, "rename" => 28,
                "wait_alertable" => 40, "sleep_ex" => 41,
                "get_version" => 43, "get_datetime" => 44, "get_meminfo" => 45,
                "get_volume_label" => 46, "chdir_parent" => 47, "kobj_enum" => 48,
                _ => return None,
            })
        } else {
            for attr in attrs {
                if attr.starts_with("syscall(") {
                    let num_str = attr.trim_start_matches("syscall(").trim_end_matches(')');
                    return num_str.parse::<u64>().ok();
                }
            }
            None
        }
    }

    fn parse_syscallnum_variant(trimmed: &str) -> Option<SyscallNumVariant> {
        let t = trimmed.trim();
        if !t.ends_with(',') { return None; }
        let t = t.trim_end_matches(',');
        if !t.contains('=') { return None; }
        let parts: Vec<&str> = t.splitn(2, '=').collect();
        if parts.len() != 2 { return None; }
        let name = parts[0].trim();
        let num_str = parts[1].trim();
        let number = num_str.parse::<u64>().ok()?;
        if name.is_empty() || !name.chars().next()?.is_uppercase() { return None; }
        Some(SyscallNumVariant { name: name.to_string(), number })
    }

    fn parse_command_entry(_trimmed: &str, lines: &[&str], line_idx: usize) -> Option<NeodosItem> {
        for l in lines.iter().take(lines.len().min(line_idx + 5)).skip(line_idx) {
            if let Some(pos) = l.find("name: \"") {
                let rest = &l[pos + 7..];
                if let Some(end) = rest.find('"') {
                    let cmd_name = rest[..end].to_string();
                    let description = l.split("description: ")
                        .nth(1).unwrap_or("Shell command")
                        .trim_matches('"').to_string();
                    return Some(NeodosItem {
                        kind: NeodosItemKind::ShellCommand,
                        name: cmd_name,
                        detail: description,
                    });
                }
            }
        }
        None
    }

    fn is_driver_state_enum(trimmed: &str) -> bool {
        let t = trimmed.trim();
        (t.starts_with("Loaded") || t.starts_with("Initialized") || t.starts_with("Registered")
            || t.starts_with("Bound") || t.starts_with("Active") || t.starts_with("Faulted")
            || t.starts_with("Unloaded") || t.starts_with("Unloading"))
            && (t.ends_with(',') || t.ends_with('}'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_code(code: &str) -> ParsedFile {
        let f = std::path::Path::new("test.rs");
        Indexer::parse_file(f, code)
    }

    #[test]
    fn test_parse_function() {
        let p = index_code("pub fn sys_write(fd: u64, buf: &[u8]) -> u64 { 0 }");
        assert_eq!(p.symbols.len(), 1);
        assert_eq!(p.symbols[0].name, "sys_write");
    }

    #[test]
    fn test_parse_struct() {
        let p = index_code("#[repr(C)]\npub struct BootInfo {\n    magic: u32,\n}");
        assert_eq!(p.symbols.len(), 1);
        assert_eq!(p.symbols[0].name, "BootInfo");
        assert_eq!(p.symbols[0].kind, SymbolKind::STRUCT);
    }

    #[test]
    fn test_parse_enum() {
        let p = index_code("pub enum ThreadState { Ready, Running, Blocked, Terminated }");
        assert_eq!(p.symbols.len(), 1);
        assert_eq!(p.symbols[0].kind, SymbolKind::ENUM);
    }

    #[test]
    fn test_parse_trait() {
        let p = index_code("pub trait FileSystem { fn read(&self) -> Result<(), ()>; }");
        assert_eq!(p.symbols.len(), 1);
        assert_eq!(p.symbols[0].kind, SymbolKind::INTERFACE);
    }

    #[test]
    fn test_parse_const() {
        let p = index_code("pub const KERNEL_VERSION: &str = \"v0.39\";\nconst MAX: usize = 16;");
        assert_eq!(p.symbols.len(), 2);
        assert!(p.symbols.iter().all(|s| s.kind == SymbolKind::CONSTANT));
    }

    #[test]
    fn test_syscall_detection() {
        let p = index_code("pub fn sys_exit(code: u64) -> ! { loop {} }");
        assert_eq!(p.neodos_items.len(), 1);
    }

    #[test]
    fn test_cap_constants() {
        let p = index_code("pub const CAP_IRQ: u64 = 1 << 0;\npub const CAP_DMA: u64 = 1 << 1;");
        assert_eq!(p.neodos_items.len(), 2);
        assert!(p.neodos_items.iter().all(|i| i.name.starts_with("CAP_")));
    }

    #[test]
    fn test_doc_comment_collection() {
        let p = index_code("/// Writes to console.\n/// Returns bytes written.\npub fn sys_write(fd: u64) -> u64 { 0 }");
        assert_eq!(p.symbols.len(), 1);
        let doc = p.symbols[0].documentation.as_deref().unwrap_or("");
        assert!(doc.contains("Writes to console"));
        assert!(doc.contains("Returns bytes written"));
    }

    #[test]
    fn test_discover_files_filters_target() {
        use std::fs;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();

        fs::write(root.join("good.rs"), "fn a() {}").ok();
        fs::write(root.join("bad.txt"), "not rust").ok();
        fs::create_dir_all(root.join("target")).ok();
        fs::write(root.join("target").join("ignored.rs"), "fn b() {}").ok();

        let mut cfg = NeodosConfig::default();
        *cfg.workspace.roots.write() = vec![root.to_path_buf()];
        cfg.workspace.max_files = 100;

        let db = Arc::new(Database::new());
        let idx = Indexer::new(db, Arc::new(cfg));
        let files = idx.discover_files();

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("good.rs"));
    }
}
