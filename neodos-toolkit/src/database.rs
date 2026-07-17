use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use lsp_types::{
    Position, Range, SymbolKind, CompletionItemKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub u64);

static NEXT_SYMBOL_ID: AtomicU64 = AtomicU64::new(1);

pub fn fresh_symbol_id() -> SymbolId {
    SymbolId(NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NeodosKind {
    Syscall(u64),
    BootPhase,
}

impl NeodosKind {
    pub fn label(&self) -> &'static str {
        match self {
            NeodosKind::Syscall(_) => "syscall",
            NeodosKind::BootPhase => "boot-phase",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub neodos_kind: Option<NeodosKind>,
    pub file: PathBuf,
    pub range: Range,
    pub selection_range: Range,
    pub parent: Option<SymbolId>,
    pub children: Vec<SymbolId>,
    pub documentation: Option<String>,
    pub detail: Option<String>,
    pub signature: Option<String>,
    pub visibility: Option<String>,
    pub attributes: Vec<String>,
    pub is_deprecated: bool,
    pub is_test: bool,
    pub syscall_number: Option<u64>,
    pub capabilities: Option<u64>,
}

impl Symbol {
    pub fn completion_item_kind(&self) -> CompletionItemKind {
        use lsp_types::SymbolKind as SK;
        if self.kind == SK::FUNCTION || self.kind == SK::METHOD {
            CompletionItemKind::FUNCTION
        } else if self.kind == SK::STRUCT || self.kind == SK::CLASS {
            CompletionItemKind::STRUCT
        } else if self.kind == SK::ENUM {
            CompletionItemKind::ENUM
        } else if self.kind == SK::INTERFACE {
            CompletionItemKind::INTERFACE
        } else if self.kind == SK::MODULE {
            CompletionItemKind::MODULE
        } else if self.kind == SK::VARIABLE {
            CompletionItemKind::VARIABLE
        } else if self.kind == SK::CONSTANT {
            CompletionItemKind::CONSTANT
        } else if self.kind == SK::TYPE_PARAMETER {
            CompletionItemKind::TYPE_PARAMETER
        } else {
            CompletionItemKind::TEXT
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub from: SymbolId,
    pub to: SymbolId,
}

#[derive(Debug, Clone)]
pub struct FileIndex {
    pub file: PathBuf,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub neodos_items: Vec<NeodosItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeodosItemKind {
    SyscallHandler,
    ShellCommand,
    BootPhaseFn,
    FileSystemImpl,
    CapabilityFlag,
    DriverState,
}

impl std::fmt::Display for NeodosItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NeodosItemKind::SyscallHandler => write!(f, "SyscallHandler"),
            NeodosItemKind::ShellCommand => write!(f, "ShellCommand"),
            NeodosItemKind::BootPhaseFn => write!(f, "BootPhaseFn"),
            NeodosItemKind::FileSystemImpl => write!(f, "FileSystemImpl"),
            NeodosItemKind::CapabilityFlag => write!(f, "CapabilityFlag"),
            NeodosItemKind::DriverState => write!(f, "DriverState"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NeodosItem {
    pub kind: NeodosItemKind,
    pub name: String,
    pub detail: String,
}

pub struct Database {
    pub symbols: DashMap<SymbolId, Symbol>,
    pub file_symbols: DashMap<PathBuf, Vec<SymbolId>>,
    pub name_index: DashMap<String, Vec<SymbolId>>,
    pub references: DashMap<SymbolId, Vec<SymbolId>>,
    pub file_indices: DashMap<PathBuf, FileIndex>,
    pub syscalls: DashMap<u64, NeodosItem>,
    pub shell_commands: DashMap<String, NeodosItem>,
    pub all_files: parking_lot::RwLock<Vec<PathBuf>>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            symbols: DashMap::new(),
            file_symbols: DashMap::new(),
            name_index: DashMap::new(),
            references: DashMap::new(),
            file_indices: DashMap::new(),
            syscalls: DashMap::new(),
            shell_commands: DashMap::new(),
            all_files: parking_lot::RwLock::new(Vec::new()),
        }
    }

    pub fn insert_symbol(&self, sym: Symbol) -> SymbolId {
        let id = sym.id;
        let name_lower = sym.name.to_lowercase();

        self.symbols.insert(id, sym.clone());
        self.name_index
            .entry(name_lower)
            .or_default()
            .push(id);

        if let Some(parent) = sym.parent
            && let Some(mut p) = self.symbols.get_mut(&parent) {
                p.children.push(id);
            }

        self.file_symbols
            .entry(sym.file.clone())
            .or_default()
            .push(id);

        id
    }

    pub fn insert_reference(&self, reference: Reference) {
        let to_id = reference.to;
        self.references
            .entry(reference.from)
            .or_default()
            .push(to_id);
    }

    pub fn lookup(&self, id: &SymbolId) -> Option<dashmap::mapref::one::Ref<'_, SymbolId, Symbol>> {
        self.symbols.get(id)
    }

    pub fn find_by_name(&self, name: &str) -> Vec<Symbol> {
        let lower = name.to_lowercase();
        let mut results = Vec::new();
        if let Some(ids) = self.name_index.get(&lower) {
            for id in ids.value() {
                if let Some(sym) = self.symbols.get(id) {
                    results.push(Symbol::clone(&*sym));
                }
            }
        }
        results
    }

    pub fn find_by_prefix(&self, prefix: &str) -> Vec<Symbol> {
        let lower = prefix.to_lowercase();
        let mut results = Vec::new();
        for entry in self.name_index.iter() {
            if entry.key().starts_with(&lower) {
                for id in entry.value() {
                    if let Some(sym) = self.symbols.get(id) {
                        results.push(Symbol::clone(&*sym));
                    }
                }
            }
        }
        results
    }

    pub fn find_at_position(&self, file: &PathBuf, pos: Position) -> Vec<Symbol> {
        let mut results = Vec::new();
        if let Some(ids) = self.file_symbols.get(file) {
            for id in ids.value() {
                if let Some(sym) = self.symbols.get(id)
                    && pos.line >= sym.range.start.line
                        && pos.line <= sym.range.end.line
                        && (pos.line != sym.range.start.line
                            || pos.character >= sym.range.start.character)
                        && (pos.line != sym.range.end.line
                            || pos.character <= sym.range.end.character)
                    {
                        results.push(Symbol::clone(&*sym));
                    }
            }
        }
        results
    }

    pub fn find_innermost_at_position(&self, file: &PathBuf, pos: Position) -> Option<Symbol> {
        let candidates = self.find_at_position(file, pos);
        candidates
            .iter()
            .max_by_key(|s| {
                let area = (s.range.end.line - s.range.start.line) * 10000
                    + (s.range.end.character - s.range.start.character);
                std::cmp::Reverse(area)
            })
            .cloned()
    }

    pub fn references_for(&self, id: &SymbolId) -> Vec<SymbolId> {
        self.references.get(id).map(|r| r.clone()).unwrap_or_default()
    }

    pub fn replace_file_index(&self, index: FileIndex) {
        let file = index.file.clone();

        if let Some(old_ids) = self.file_symbols.get(&file) {
            for id in old_ids.value().iter() {
                self.symbols.remove(id);
                self.references.remove(id);
            }
        }
        self.file_symbols.remove(&file);
        self.name_index.retain(|_, ids| {
            ids.retain(|id| self.symbols.contains_key(id));
            !ids.is_empty()
        });

        self.file_indices.insert(file.clone(), index.clone());
        for sym in &index.symbols {
            self.insert_symbol(sym.clone());
        }
        for rf in &index.references {
            self.insert_reference(rf.clone());
        }
        for item in &index.neodos_items {
            match item.kind {
                NeodosItemKind::SyscallHandler => {
                    if let Some(num) = item.name.strip_prefix("SyscallNum::")
                        && let Some(num) = num.split('(').next().and_then(|s| s.trim().parse::<u64>().ok()) {
                            self.syscalls.insert(num, item.clone());
                        }
                }
                NeodosItemKind::ShellCommand => {
                    self.shell_commands.insert(item.name.clone(), item.clone());
                }
                _ => {}
            }
        }
    }

    pub fn document_symbols(&self, file: &PathBuf) -> Vec<Symbol> {
        let mut results = Vec::new();
        if let Some(ids) = self.file_symbols.get(file) {
            for id in ids.value() {
                if let Some(sym) = self.symbols.get(id) {
                    results.push(Symbol::clone(&*sym));
                }
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_range() -> Range {
        Range {
            start: Position { line: 0, character: 0 },
            end: Position { line: 1, character: 0 },
        }
    }

    fn make_sym(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            id: fresh_symbol_id(),
            name: name.to_string(),
            kind,
            neodos_kind: None,
            file: PathBuf::from("test.rs"),
            range: make_range(),
            selection_range: make_range(),
            parent: None,
            children: Vec::new(),
            documentation: None,
            detail: None,
            signature: None,
            visibility: None,
            attributes: Vec::new(),
            is_deprecated: false,
            is_test: false,
            syscall_number: None,
            capabilities: None,
        }
    }

    #[test]
    fn test_insert_and_lookup() {
        let db = Database::new();
        let s = make_sym("test_fn", SymbolKind::FUNCTION);
        let id = db.insert_symbol(s);
        assert!(db.lookup(&id).is_some());
        assert_eq!(db.lookup(&id).unwrap().name, "test_fn");
    }

    #[test]
    fn test_find_by_name() {
        let db = Database::new();
        let s = make_sym("MyStruct", SymbolKind::STRUCT);
        db.insert_symbol(s);
        let found = db.find_by_name("mystruct");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "MyStruct");
    }

    #[test]
    fn test_find_by_prefix() {
        let db = Database::new();
        db.insert_symbol(make_sym("sys_write", SymbolKind::FUNCTION));
        db.insert_symbol(make_sym("sys_read", SymbolKind::FUNCTION));
        db.insert_symbol(make_sym("sys_exit", SymbolKind::FUNCTION));
        db.insert_symbol(make_sym("other", SymbolKind::VARIABLE));

        let found = db.find_by_prefix("sys_");
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn test_replace_file_index_removes_old() {
        let db = Database::new();
        let f = PathBuf::from("replace.rs");
        let range = make_range();

        let old = FileIndex {
            file: f.clone(),
            symbols: vec![Symbol {
                id: fresh_symbol_id(),
                name: "gone".into(),
                kind: SymbolKind::FUNCTION,
                neodos_kind: None, file: f.clone(),
                range, selection_range: range,
                parent: None, children: vec![],
                documentation: None, detail: None, signature: None,
                visibility: None, attributes: vec![],
                is_deprecated: false, is_test: false,
                syscall_number: None, capabilities: None,
            }],
            references: vec![],
            neodos_items: vec![],
        };
        db.replace_file_index(old);
        assert_eq!(db.find_by_name("gone").len(), 1);

        let new = FileIndex {
            file: f.clone(),
            symbols: vec![Symbol {
                id: fresh_symbol_id(),
                name: "new_one".into(),
                kind: SymbolKind::FUNCTION,
                neodos_kind: None, file: f.clone(),
                range, selection_range: range,
                parent: None, children: vec![],
                documentation: None, detail: None, signature: None,
                visibility: None, attributes: vec![],
                is_deprecated: false, is_test: false,
                syscall_number: None, capabilities: None,
            }],
            references: vec![],
            neodos_items: vec![],
        };
        db.replace_file_index(new);
        assert!(db.find_by_name("gone").is_empty());
        assert_eq!(db.find_by_name("new_one").len(), 1);
    }
}
