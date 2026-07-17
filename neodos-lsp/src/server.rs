use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crossbeam::channel::{Receiver, Sender};
use lsp_types::*;

use neodos_toolkit::config::NeodosConfig;
use neodos_toolkit::database::Database;
use neodos_toolkit::indexer::Indexer;

use crate::handlers::{self, LspHandlers};

#[derive(Debug)]
struct JsonRpcMessage {
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

pub struct LspServer {
    config: Arc<NeodosConfig>,
    db: Arc<Database>,
    handlers: LspHandlers,
    diag_tx: Sender<(PathBuf, Vec<Diagnostic>)>,
    diag_rx: Receiver<(PathBuf, Vec<Diagnostic>)>,
    _client_caps: ClientCapabilities,
}

impl LspServer {
    pub fn new(config: Arc<NeodosConfig>) -> Self {
        let db = Arc::new(Database::new());
        let (dtx, drx) = crossbeam::channel::unbounded();

        Self {
            db: db.clone(),
            handlers: LspHandlers::new(db, config.clone()),
            config,
            diag_tx: dtx,
            diag_rx: drx,
            _client_caps: ClientCapabilities::default(),
        }
    }

    pub fn run(&mut self) -> Result<(), String> {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        let mut stdout = std::io::stdout().lock();

        log::info!("LSP server running, waiting for initialize...");

        loop {
            let msg = match read_message(&mut reader) {
                Ok(Some(msg)) => msg,
                Ok(None) => {
                    log::info!("client closed connection (EOF)");
                    break;
                }
                Err(e) => {
                    log::error!("error reading message: {e}");
                    break;
                }
            };

            log::trace!(">>> {} {:?}", msg.method, msg.id);

            match msg.method.as_str() {
                "exit" => {
                    log::info!("received exit notification, shutting down");
                    break;
                }

                "initialize" => {
                    let result = self.handle_initialize(msg.params);
                    let caps = serde_json::to_value(result).unwrap_or_default();
                    let response = make_response(msg.id, caps);
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "initialized" => {
                    log::info!("client initialized");
                    self.start_background_indexing();
                }

                "shutdown" => {
                    log::info!("received shutdown request");
                    let response = make_response(msg.id, serde_json::Value::Null);
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "textDocument/didOpen" => {
                    if let Some(params) = msg.params
                        && let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(params) {
                            self.handlers.on_did_open(p);
                        }
                }

                "textDocument/didChange" => {
                    if let Some(params) = msg.params
                        && let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(params) {
                            let change_uri = p.text_document.uri.clone();
                            self.handlers.on_did_change(p);
                            let path = handlers::uri_to_path(&change_uri);
                            self.request_diagnostics(path);
                        }
                }

                "textDocument/didSave" => {
                    if let Some(params) = msg.params
                        && let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(params) {
                            self.handlers.on_did_save(p);
                        }
                }

                "textDocument/didClose" => {
                    if let Some(params) = msg.params
                        && let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(params) {
                            self.handlers.on_did_close(p);
                        }
                }

                "workspace/didChangeWatchedFiles" => {}

                "textDocument/completion" => {
                    let result = msg.params.and_then(|params| {
                        serde_json::from_value::<CompletionParams>(params).ok()
                            .and_then(|p| self.handlers.completion(p))
                    });
                    let response = make_response(msg.id, serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "textDocument/definition" => {
                    let result = msg.params.and_then(|params| {
                        serde_json::from_value::<GotoDefinitionParams>(params).ok()
                            .and_then(|p| self.handlers.goto_definition(p))
                    });
                    let response = make_response(msg.id, serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "textDocument/references" => {
                    let result = msg.params.and_then(|params| {
                        serde_json::from_value::<ReferenceParams>(params).ok()
                            .and_then(|p| self.handlers.find_references(p))
                    });
                    let response = make_response(msg.id, serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "textDocument/hover" => {
                    let result = msg.params.and_then(|params| {
                        serde_json::from_value::<HoverParams>(params).ok()
                            .and_then(|p| self.handlers.hover(p))
                    });
                    let response = make_response(msg.id, serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "textDocument/rename" => {
                    let result = msg.params.and_then(|params| {
                        serde_json::from_value::<RenameParams>(params).ok()
                            .and_then(|p| self.handlers.rename(p))
                    });
                    let response = make_response(msg.id, serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                "textDocument/documentSymbol" => {
                    let result = msg.params.and_then(|params| {
                        serde_json::from_value::<DocumentSymbolParams>(params).ok()
                            .and_then(|p| self.handlers.document_symbols(p))
                    });
                    let response = make_response(msg.id, serde_json::to_value(result).unwrap_or(serde_json::Value::Null));
                    write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                }

                _ => {
                    log::warn!("unhandled method: {} (id: {:?})", msg.method, msg.id);
                    if msg.id.is_some() {
                        let error = serde_json::json!({
                            "code": -32601,
                            "message": format!("method not found: {}", msg.method),
                        });
                        let response = make_error_response(msg.id, error);
                        write_message(&mut stdout, &response).map_err(|e| e.to_string())?;
                    }
                }
            }

            self.flush_diagnostics(&mut stdout)
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn handle_initialize(&mut self, params: Option<serde_json::Value>) -> InitializeResult {
        if let Some(ref params) = params
            && let Ok(p) = serde_json::from_value::<InitializeParams>(params.clone()) {
                log::info!(
                    "client: {} {}",
                    p.client_info.as_ref().map(|i| i.name.as_str()).unwrap_or("unknown"),
                    p.client_info.as_ref().and_then(|i| i.version.as_deref()).unwrap_or(""),
                );

                #[allow(deprecated)]
                let root_from_deprecated: Option<lsp_types::Uri> = p.root_uri.clone().or_else(|| {
                    p.root_path.as_ref().and_then(|rp| {
                        url::Url::from_file_path(rp).ok()
                            .and_then(|u| u.as_str().parse::<lsp_types::Uri>().ok())
                    })
                });
                if let Some(folders) = p.workspace_folders {
                    let roots: Vec<PathBuf> = folders
                        .iter()
                        .filter_map(|f| {
                            url::Url::parse(f.uri.as_str())
                                .ok()
                                .and_then(|u| u.to_file_path().ok())
                        })
                        .collect();
                    if !roots.is_empty() {
                        *self.config.workspace.roots.write() = roots;
                    }
                } else if let Some(uri) = root_from_deprecated {
                    if let Some(path) = url::Url::parse(uri.as_str()).ok().and_then(|u| u.to_file_path().ok()) {
                        *self.config.workspace.roots.write() = vec![path];
                    }
                }

                self._client_caps = p.capabilities;
            }

        InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        will_save: None,
                        will_save_wait_until: None,
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ".".into(), "::".into(), "_".into(),
                    ]),
                    all_commit_characters: None,
                    resolve_provider: None,
                    work_done_progress_options: Default::default(),
                    completion_item: None,
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                rename_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "neodos-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        }
    }

    fn start_background_indexing(&self) {
        let db = self.db.clone();
        let config = self.config.clone();
        let diag_tx = self.diag_tx.clone();
        let db2 = self.db.clone();
        let config2 = self.config.clone();

        thread::Builder::new()
            .name("neodos-lsp-indexer".into())
            .spawn(move || {
                log::info!("background indexing started");

                let indexer = Indexer::new(db.clone(), config.clone());
                let files = indexer.discover_files();

                let count = indexer.index_workspace(&files);

                log::info!("indexed {} symbols in {} files", count, files.len());

                *db.all_files.write() = files.clone();
                log::info!("background indexing complete");

                let wm = neodos_toolkit::workspace::WorkspaceManager::new(config.clone());
                wm.register_files(&files);

                loop {
                    thread::sleep(std::time::Duration::from_secs(2));

                    let events = wm.poll_for_changes();
                    for (_path, event) in events {
                        match event {
                            neodos_toolkit::workspace::FileEvent::Created(p)
                            | neodos_toolkit::workspace::FileEvent::Modified(p) => {
                                if let Ok(content) = std::fs::read_to_string(&p) {
                                    let parsed = Indexer::parse_file(&p, &content);
                                    let fi = neodos_toolkit::database::FileIndex {
                                        file: p.clone(),
                                        symbols: parsed.symbols,
                                        references: parsed.references,
                                        neodos_items: parsed.neodos_items,
                                    };
                                    db.replace_file_index(fi);

                                    let handlers = LspHandlers::new(db2.clone(), config2.clone());
                                    let diags = handlers.diagnostics(&p);
                                    diag_tx.send((p, diags)).ok();
                                }
                            }
                            neodos_toolkit::workspace::FileEvent::Deleted(p) => {
                                db.replace_file_index(neodos_toolkit::database::FileIndex {
                                    file: p,
                                    symbols: vec![],
                                    references: vec![],
                                    neodos_items: vec![],
                                });
                            }
                            neodos_toolkit::workspace::FileEvent::FullRescan => {
                                let files = indexer.discover_files();
                                indexer.index_workspace(&files);
                            }
                        }
                    }
                }
            })
            .expect("failed to spawn indexer thread");
    }

    fn request_diagnostics(&self, path: PathBuf) {
        let diags = self.handlers.diagnostics(&path);
        self.diag_tx.send((path, diags)).ok();
    }

    fn flush_diagnostics(&self, writer: &mut impl Write) -> Result<(), std::io::Error> {
        while let Ok((path, diags)) = self.diag_rx.try_recv() {
            let params = PublishDiagnosticsParams {
                uri: handlers::path_to_uri(&path),
                diagnostics: diags,
                version: None,
            };
            let notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": params,
            });
            write_message_inner(writer, &notification)?;
        }
        Ok(())
    }
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<JsonRpcMessage>, String> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut header = String::new();
        let bytes = reader
            .read_line(&mut header)
            .map_err(|e| format!("read header: {e}"))?;
        if bytes == 0 {
            return Ok(None);
        }

        let header = header.trim();
        if header.is_empty() {
            break;
        }

        if let Some(val) = header
            .to_lowercase()
            .strip_prefix("content-length:")
        {
            content_length = Some(val.trim().parse().map_err(|e| format!("invalid Content-Length: {e}"))?);
        }
    }

    let len = content_length.ok_or("missing Content-Length header")?;

    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .map_err(|e| format!("read body ({len} bytes): {e}"))?;

    let body_str =
        String::from_utf8(body).map_err(|e| format!("invalid UTF-8: {e}"))?;

    let json: serde_json::Value =
        serde_json::from_str(&body_str).map_err(|e| format!("invalid JSON: {e} ({body_str:?})"))?;

    let id = json.get("id").cloned();
    let method = json
        .get("method")
        .and_then(|v| v.as_str())
        .ok_or("missing method field")?
        .to_string();
    let params = json.get("params").cloned();

    log::trace!("<<< {} (id={:?})", method, id);
    if log::log_enabled!(log::Level::Trace)
        && let Some(ref p) = params {
            let s = serde_json::to_string(p).unwrap_or_default();
            if s.len() < 200 {
                log::trace!("    params: {s}");
            }
        }

    Ok(Some(JsonRpcMessage { id, method, params }))
}

fn write_message(writer: &mut impl Write, value: &serde_json::Value) -> Result<(), std::io::Error> {
    write_message_inner(writer, value)
}

fn write_message_inner(writer: &mut impl Write, value: &serde_json::Value) -> Result<(), std::io::Error> {
    let body = serde_json::to_string(value).map_err(|e| {
        std::io::Error::other(format!("serialize: {e}"))
    })?;

    if log::log_enabled!(log::Level::Trace) {
        let s = if body.len() < 300 {
            body.clone()
        } else {
            format!("{}...({} bytes)", &body[..200], body.len())
        };
        log::trace!(">>> {}", s);
    }

    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()?;
    Ok(())
}

fn make_response(id: Option<serde_json::Value>, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn make_error_response(
    id: Option<serde_json::Value>,
    error: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": error,
    })
}
