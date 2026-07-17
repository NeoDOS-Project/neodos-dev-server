use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lsp_types::*;

use neodos_toolkit::cache::DocumentCache;
use neodos_toolkit::config::NeodosConfig;
use neodos_toolkit::database::{Database, Symbol};
use neodos_toolkit::indexer::Indexer;

pub fn path_to_uri(path: &PathBuf) -> lsp_types::Uri {
    let abs = if path.is_relative() {
        std::env::current_dir().unwrap_or_default().join(path)
    } else {
        path.clone()
    };
    let url = url::Url::from_file_path(&abs).expect("valid file path");
    url.as_str().parse::<lsp_types::Uri>().expect("valid lsp URI")
}

pub fn uri_to_path(uri: &lsp_types::Uri) -> PathBuf {
    let url_str = uri.as_str();
    let url = url::Url::parse(url_str).expect("valid URL");
    url.to_file_path().expect("valid file:// URI")
}

pub struct LspHandlers {
    pub db: Arc<Database>,
    pub cache: Arc<DocumentCache>,
}

impl LspHandlers {
    pub fn new(
        db: Arc<Database>,
        config: Arc<NeodosConfig>,
    ) -> Self {
        let cache = Arc::new(DocumentCache::new(config.cache.documents));
        Self { db, cache }
    }

    pub fn on_did_open(&self, params: DidOpenTextDocumentParams) {
        let path = uri_to_path(&params.text_document.uri);
        let version = params.text_document.version as i64;
        let content = &params.text_document.text;

        log::info!("didOpen: {} (v{})", path.display(), version);

        let parsed = Indexer::parse_file(&path, content);
        let parsed_clone = parsed.clone();

        self.cache.insert(path.clone(), content.clone(), version, parsed_clone);

        let file_index = neodos_toolkit::database::FileIndex {
            file: path.clone(),
            symbols: parsed.symbols,
            references: parsed.references,
            neodos_items: parsed.neodos_items,
        };
        self.db.replace_file_index(file_index);
    }

    pub fn on_did_change(&self, params: DidChangeTextDocumentParams) {
        let path = uri_to_path(&params.text_document.uri);
        let version = params.text_document.version as i64;

        log::trace!("didChange: {} (v{})", path.display(), version);

        if let Some(change) = params.content_changes.into_iter().last() {
            let content = change.text;

            let parsed = Indexer::parse_file(&path, &content);
            let parsed_clone = parsed.clone();

            self.cache.insert(path.clone(), content, version, parsed_clone);

            let file_index = neodos_toolkit::database::FileIndex {
                file: path.clone(),
                symbols: parsed.symbols,
                references: parsed.references,
                neodos_items: parsed.neodos_items,
            };
            self.db.replace_file_index(file_index);
        }
    }

    pub fn on_did_save(&self, _params: DidSaveTextDocumentParams) {
        log::trace!("didSave");
    }

    pub fn on_did_close(&self, params: DidCloseTextDocumentParams) {
        let path = uri_to_path(&params.text_document.uri);
        log::info!("didClose: {}", path.display());
        self.cache.remove(&path);
    }

    pub fn completion(&self, params: CompletionParams) -> Option<CompletionResponse> {
        let path = uri_to_path(&params.text_document_position.text_document.uri);
        let pos = params.text_document_position.position;
        log::debug!("completion at {}:{},{}", path.display(), pos.line, pos.character);

        let prefix = self.word_at_position(&path, pos);
        log::trace!("completion prefix: '{:?}'", prefix);

        let mut items: Vec<CompletionItem> = Vec::new();

        if let Some(ref p) = prefix {
            for sym in self.db.find_by_prefix(p) {
                items.push(Self::sym_to_completion(&sym));
            }
        }

        if prefix.as_deref() == Some("sys_") || prefix.as_deref().is_some_and(|p| p.starts_with("sys_")) {
            for entry in self.db.syscalls.iter() {
                let (num, item) = entry.pair();
                items.push(CompletionItem {
                    label: format!("sys_{}", item.name),
                    detail: Some(format!("syscall #{num} — {}", item.detail)),
                    kind: Some(CompletionItemKind::FUNCTION),
                    insert_text: Some(format!("sys_{}", item.name)),
                    ..Default::default()
                });
            }
        }

        if prefix.as_deref() == Some("") || prefix.as_deref().is_some_and(|p| p.len() <= 3) {
            for entry in self.db.shell_commands.iter() {
                items.push(CompletionItem {
                    label: entry.key().clone(),
                    detail: Some(entry.value().detail.clone()),
                    kind: Some(CompletionItemKind::FUNCTION),
                    insert_text: Some(entry.key().clone()),
                    ..Default::default()
                });
            }
        }

        if prefix.as_deref().is_some_and(|p| p.starts_with("CAP_")) {
            for sym in self.db.find_by_prefix("CAP_") {
                items.push(Self::sym_to_completion(&sym));
            }
        }

        items.sort_by(|a, b| {
            let a_exact = prefix.as_ref().is_some_and(|p| a.label.eq_ignore_ascii_case(p));
            let b_exact = prefix.as_ref().is_some_and(|p| b.label.eq_ignore_ascii_case(p));
            a_exact.cmp(&b_exact).reverse().then_with(|| a.label.cmp(&b.label))
        });
        items.dedup_by(|a, b| a.label == b.label);

        log::debug!("completion: {} items", items.len());
        Some(CompletionResponse::Array(items.into_iter().take(50).collect()))
    }

    pub fn goto_definition(&self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let path = uri_to_path(&params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        log::debug!("goto-def at {}:{},{}", path.display(), pos.line, pos.character);

        let word = self.word_at_position(&path, pos)?;
        let mut results: Vec<Location> = Vec::new();

        for sym in self.db.find_by_name(&word) {
            results.push(Location {
                uri: path_to_uri(&sym.file),
                range: sym.selection_range,
            });
        }

        if results.is_empty() && word.len() >= 2 {
            for sym in self.db.find_by_prefix(&word).iter().take(5) {
                results.push(Location {
                    uri: path_to_uri(&sym.file),
                    range: sym.selection_range,
                });
            }
        }

        log::debug!("goto-def: {} results", results.len());
        if results.is_empty() { None } else { Some(GotoDefinitionResponse::Array(results)) }
    }

    pub fn find_references(&self, params: ReferenceParams) -> Option<Vec<Location>> {
        let path = uri_to_path(&params.text_document_position.text_document.uri);
        let pos = params.text_document_position.position;
        log::debug!("find-references at {}:{},{}", path.display(), pos.line, pos.character);

        let word = self.word_at_position(&path, pos)?;
        let defs = self.db.find_by_name(&word);
        if defs.is_empty() { return Some(Vec::new()); }

        let target_id = defs[0].id;
        let ref_ids = self.db.references_for(&target_id);

        let mut locations: Vec<Location> = Vec::new();
        for ref_id in ref_ids {
            if let Some(sym) = self.db.lookup(&ref_id) {
                locations.push(Location {
                    uri: path_to_uri(&sym.file),
                    range: sym.range,
                });
            }
        }

        if params.context.include_declaration {
            locations.push(Location {
                uri: path_to_uri(&defs[0].file),
                range: defs[0].selection_range,
            });
        }

        log::debug!("find-references: {} locations", locations.len());
        Some(locations)
    }

    pub fn hover(&self, params: HoverParams) -> Option<Hover> {
        let path = uri_to_path(&params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        log::debug!("hover at {}:{},{}", path.display(), pos.line, pos.character);

        let sym = self.db.find_innermost_at_position(&path, pos)?;

        let mut contents: Vec<MarkedString> = Vec::new();

        if let Some(ref sig) = sym.signature {
            contents.push(MarkedString::LanguageString(LanguageString {
                language: "rust".into(),
                value: sig.clone(),
            }));
        } else {
            let kind_label = symbol_kind_label(sym.kind);
            contents.push(MarkedString::String(format!("**{kind_label}** `{}`", sym.name)));
        }

        if let Some(ref ndk) = sym.neodos_kind {
            contents.push(MarkedString::String(format!("**NeoDOS {}**", ndk.label())));
            if let Some(num) = sym.syscall_number {
                contents.push(MarkedString::String(format!("Syscall #{num}")));
            }
            if let Some(ref caps) = sym.capabilities {
                contents.push(MarkedString::String(format!("Capabilities: 0x{caps:x}")));
            }
        }

        if let Some(ref vis) = sym.visibility {
            contents.push(MarkedString::String(format!("**Visibility:** {vis}")));
        }

        if let Some(ref doc) = sym.documentation {
            contents.push(MarkedString::String(doc.clone()));
        }

        contents.push(MarkedString::String(format!(
            "{}:{}:{}",
            sym.file.file_name().unwrap_or_default().to_string_lossy(),
            sym.range.start.line + 1,
            sym.range.start.character + 1,
        )));

        Some(Hover {
            contents: HoverContents::Array(contents),
            range: Some(sym.range),
        })
    }

    pub fn rename(&self, params: RenameParams) -> Option<WorkspaceEdit> {
        let path = uri_to_path(&params.text_document_position.text_document.uri);
        let pos = params.text_document_position.position;
        let new_name = &params.new_name;

        log::info!("rename at {}:{},{} -> '{}'", path.display(), pos.line, pos.character, new_name);

        let sym = self.db.find_innermost_at_position(&path, pos)?;
        let target_id = sym.id;
        let ref_ids = self.db.references_for(&target_id);

        let mut changes: HashMap<lsp_types::Uri, Vec<TextEdit>> = HashMap::new();

        let def_uri = path_to_uri(&sym.file);
        changes.entry(def_uri).or_default().push(TextEdit {
            range: sym.selection_range,
            new_text: new_name.clone(),
        });

        for ref_id in ref_ids {
            if let Some(ref_sym) = self.db.lookup(&ref_id) {
                let u = path_to_uri(&ref_sym.file);
                changes.entry(u).or_default().push(TextEdit {
                    range: ref_sym.selection_range,
                    new_text: new_name.clone(),
                });
            }
        }

        Some(WorkspaceEdit {
            changes: Some(changes.into_iter().collect()),
            document_changes: None,
            change_annotations: None,
        })
    }

    pub fn document_symbols(&self, params: DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
        let path = uri_to_path(&params.text_document.uri);
        log::debug!("document-symbols for {}", path.display());

        let symbols = self.db.document_symbols(&path);
        if symbols.is_empty() {
            return Some(DocumentSymbolResponse::Flat(Vec::new()));
        }

        let top_level: Vec<Symbol> = symbols.iter().filter(|s| s.parent.is_none()).cloned().collect();
        let result: Vec<DocumentSymbol> = top_level.into_iter()
            .map(|s| self.symbol_to_document_symbol(&s, &symbols))
            .collect();

        Some(DocumentSymbolResponse::Nested(result))
    }

    pub fn diagnostics(&self, path: &PathBuf) -> Vec<Diagnostic> {
        log::trace!("diagnostics for {}", path.display());
        let mut diags: Vec<Diagnostic> = Vec::new();

        let content = self.cache.get_source(path)
            .or_else(|| std::fs::read_to_string(path).ok());

        let content = match content {
            Some(c) => c,
            None => return diags,
        };

        let mut open_braces: i32 = 0;
        let mut open_parens: i32 = 0;
        for ch in content.chars() {
            match ch {
                '{' => open_braces += 1,
                '}' => open_braces -= 1,
                '(' => open_parens += 1,
                ')' => open_parens -= 1,
                _ => {}
            }
        }

        let last_line = content.lines().count().saturating_sub(1) as u32;

        if open_braces != 0 {
            diags.push(Diagnostic {
                range: Range {
                    start: Position { line: last_line, character: 0 },
                    end: Position { line: last_line, character: 0 },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: format!("unbalanced braces: {open_braces} unclosed"),
                source: Some("neodos-lsp".into()),
                ..Default::default()
            });
        }
        if open_parens != 0 {
            diags.push(Diagnostic {
                range: Range {
                    start: Position { line: last_line, character: 0 },
                    end: Position { line: last_line, character: 0 },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: format!("unbalanced parentheses: {open_parens} unclosed"),
                source: Some("neodos-lsp".into()),
                ..Default::default()
            });
        }

        diags
    }

    fn word_at_position(&self, path: &PathBuf, pos: Position) -> Option<String> {
        let content = self.cache.get_source(path)
            .or_else(|| std::fs::read_to_string(path).ok())?;

        let line = content.lines().nth(pos.line as usize)?;
        let col = pos.character as usize;

        if col >= line.len() { return None; }

        let before: String = line[..col]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':' || *c == '#' || *c == '!')
            .collect();
        let after: String = line[col..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':' || *c == '!' || *c == '?')
            .collect();

        let word = format!("{}{}", before.chars().rev().collect::<String>(), after);
        if word.is_empty() { None } else { Some(word) }
    }

    fn sym_to_completion(sym: &Symbol) -> CompletionItem {
        CompletionItem {
            label: sym.name.clone(),
            kind: Some(sym.completion_item_kind()),
            detail: sym.detail.clone().or_else(|| {
                sym.neodos_kind.map(|k| format!("[{}]", k.label()))
            }),
            documentation: sym.documentation.clone().map(Documentation::String),
            insert_text: Some(sym.name.clone()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        }
    }

    fn symbol_to_document_symbol(&self, sym: &Symbol, all_symbols: &[Symbol]) -> DocumentSymbol {
        let children: Vec<DocumentSymbol> = all_symbols
            .iter()
            .filter(|s| s.parent == Some(sym.id))
            .map(|s| self.symbol_to_document_symbol(s, all_symbols))
            .collect();

        let tags = if sym.is_deprecated {
            Some(vec![SymbolTag::DEPRECATED])
        } else {
            None
        };

        #[allow(deprecated)]
        DocumentSymbol {
            name: sym.name.clone(),
            detail: sym.detail.clone().or_else(|| sym.neodos_kind.as_ref().map(|k| k.label().to_string())),
            kind: sym.kind,
            tags,
            deprecated: None,
            range: sym.range,
            selection_range: sym.selection_range,
            children: if children.is_empty() { None } else { Some(children) },
        }
    }
}

fn symbol_kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FUNCTION => "function",
        SymbolKind::MODULE => "module",
        SymbolKind::STRUCT => "struct",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "trait",
        SymbolKind::METHOD => "method",
        SymbolKind::CONSTANT => "constant",
        SymbolKind::VARIABLE => "variable",
        SymbolKind::TYPE_PARAMETER => "type",
        SymbolKind::OBJECT => "impl",
        _ => "symbol",
    }
}
