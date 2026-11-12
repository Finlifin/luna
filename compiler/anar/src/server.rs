//! LSP 3.17 JSON-RPC 2.0 server dispatcher for `anar`.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::analysis::{LspDiagnostic, WorkspaceAnalyzer};
use crate::index::{semantic_token_modifier, semantic_token_type};
use crate::json::JsonValue;
use crate::position::{LspPosition, path_to_uri, uri_to_path};
use crate::rpc::{
    ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, RequestId, RpcMessage,
    read_rpc_message, send_error, send_notification, send_response,
};

pub const ANAR_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct LanguageServer {
    pub analyzer: WorkspaceAnalyzer,
    initialized: bool,
    shutdown_requested: bool,
    cancelled_ids: HashSet<RequestId>,
    published_files: HashSet<PathBuf>,
    format_enabled: bool,
}

impl LanguageServer {
    pub fn new(sysroot_override: Option<PathBuf>) -> Self {
        Self {
            analyzer: WorkspaceAnalyzer::new(sysroot_override),
            initialized: false,
            shutdown_requested: false,
            cancelled_ids: HashSet::new(),
            published_files: HashSet::new(),
            format_enabled: true,
        }
    }

    /// Run the main JSON-RPC message loop until `exit` or EOF.
    /// Returns the process exit code (`0` after clean `shutdown`, `1` if `exit` arrived without `shutdown`).
    pub fn run<R: BufRead, W: Write>(&mut self, reader: &mut R, writer: &mut W) -> io::Result<i32> {
        while let Some(msg) = read_rpc_message(reader)? {
            match msg {
                RpcMessage::Notification { method, params } => {
                    if method == "exit" {
                        return Ok(if self.shutdown_requested { 0 } else { 1 });
                    }
                    self.handle_notification(&method, &params, writer)?;
                }
                RpcMessage::Request { id, method, params } => {
                    if self.cancelled_ids.remove(&id) {
                        send_error(writer, id, -32800, "request cancelled")?;
                        continue;
                    }
                    if self.shutdown_requested && method != "exit" {
                        send_error(writer, id, ERR_INVALID_REQUEST, "server is shutting down")?;
                        continue;
                    }
                    self.handle_request(id, &method, &params, writer)?;
                }
                RpcMessage::Response { .. } => {}
            }
        }
        Ok(0)
    }

    fn handle_notification<W: Write>(
        &mut self,
        method: &str,
        params: &JsonValue,
        writer: &mut W,
    ) -> io::Result<()> {
        match method {
            "initialized" => {
                self.initialized = true;
            }
            "$/cancelRequest" => {
                if let Some(id_val) = params.get("id")
                    && let Some(id) = RequestId::from_json(id_val)
                {
                    self.cancelled_ids.insert(id);
                }
            }
            "$/setTrace" => {}
            "workspace/didChangeConfiguration" => {
                if let Some(settings) = params.get("settings").and_then(|s| s.get("flurry")) {
                    if let Some(sr) = settings
                        .get("anar")
                        .and_then(|a| a.get_str("sysroot"))
                        .or_else(|| settings.get_str("sysroot"))
                    {
                        let trimmed = sr.trim();
                        if trimmed.is_empty() {
                            self.analyzer.set_sysroot_override(None);
                        } else {
                            self.analyzer
                                .set_sysroot_override(Some(PathBuf::from(trimmed)));
                        }
                    }
                    if let Some(fmt_en) = settings.get("format").and_then(|f| f.get_bool("enable"))
                    {
                        self.format_enabled = fmt_en;
                    }
                }
                self.analyzer.invalidate_all();
                self.republish_open_diagnostics(writer)?;
            }
            "workspace/didChangeWorkspaceFolders" => {
                if let Some(event) = params.get("event") {
                    if let Some(removed) = event.get_array("removed") {
                        for rem in removed {
                            if let Some(uri) = rem.get_str("uri")
                                && let Some(p) = uri_to_path(uri)
                            {
                                self.analyzer.workspace_roots.retain(|r| r != &p);
                            }
                        }
                    }
                    if let Some(added) = event.get_array("added") {
                        for add in added {
                            if let Some(uri) = add.get_str("uri")
                                && let Some(p) = uri_to_path(uri)
                                && !self.analyzer.workspace_roots.contains(&p)
                            {
                                self.analyzer.workspace_roots.push(p);
                            }
                        }
                    }
                }
                self.analyzer.invalidate_all();
            }
            "workspace/didChangeWatchedFiles" => {
                self.analyzer.invalidate_all();
                self.republish_open_diagnostics(writer)?;
            }
            "textDocument/didOpen" => {
                if let Some(td) = params.get("textDocument")
                    && let Some(uri) = td.get_str("uri")
                    && let Some(text) = td.get_str("text")
                {
                    let version = td.get_i64("version").unwrap_or(0);
                    if let Some(path) = self.analyzer.open_document(uri, version, text.to_owned()) {
                        self.publish_package_diagnostics(&path, writer)?;
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(td) = params.get("textDocument")
                    && let Some(uri) = td.get_str("uri")
                    && let Some(changes) = params.get_array("contentChanges")
                {
                    let version = td.get_i64("version").unwrap_or(0);
                    if let Some(path) = self.analyzer.change_document(uri, version, changes) {
                        self.publish_package_diagnostics(&path, writer)?;
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(td) = params.get("textDocument")
                    && let Some(uri) = td.get_str("uri")
                {
                    let text = params.get_str("text").map(str::to_owned);
                    if let Some(path) = self.analyzer.save_document(uri, text) {
                        self.publish_package_diagnostics(&path, writer)?;
                    }
                }
            }
            "textDocument/didClose" => {
                if let Some(td) = params.get("textDocument")
                    && let Some(uri) = td.get_str("uri")
                    && let Some(path) = self.analyzer.close_document(uri)
                {
                    if path.exists() {
                        self.publish_package_diagnostics(&path, writer)?;
                    } else {
                        self.publish_file_diagnostics(&path, &[], writer)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_request<W: Write>(
        &mut self,
        id: RequestId,
        method: &str,
        params: &JsonValue,
        writer: &mut W,
    ) -> io::Result<()> {
        match method {
            "initialize" => {
                self.apply_initialize_params(params);
                let result = build_initialize_result();
                send_response(writer, id, result)?;
            }
            "shutdown" => {
                self.shutdown_requested = true;
                send_response(writer, id, JsonValue::Null)?;
            }
            "textDocument/definition" | "textDocument/declaration" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let locs = self.analyzer.goto_definition(&path, pos);
                let resp = if locs.is_empty() {
                    JsonValue::Null
                } else {
                    JsonValue::Array(locs)
                };
                send_response(writer, id, resp)?;
            }
            "textDocument/references" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let include_decl = params
                    .get("context")
                    .and_then(|c| c.get_bool("includeDeclaration"))
                    .unwrap_or(true);
                let refs = self.analyzer.find_references(&path, pos, include_decl);
                send_response(writer, id, JsonValue::Array(refs))?;
            }
            "textDocument/documentHighlight" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let hl = self.analyzer.document_highlights(&path, pos);
                send_response(writer, id, JsonValue::Array(hl))?;
            }
            "textDocument/hover" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let hover = self.analyzer.hover(&path, pos).unwrap_or(JsonValue::Null);
                send_response(writer, id, hover)?;
            }
            "textDocument/completion" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let items = self.analyzer.complete(&path, pos);
                let result = JsonValue::object([
                    ("isIncomplete", JsonValue::Bool(false)),
                    ("items", JsonValue::Array(items)),
                ]);
                send_response(writer, id, result)?;
            }
            "textDocument/signatureHelp" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let sh = self
                    .analyzer
                    .signature_help(&path, pos)
                    .unwrap_or(JsonValue::Null);
                send_response(writer, id, sh)?;
            }
            "textDocument/documentSymbol" => {
                let Some(path) = extract_doc_path(params) else {
                    return send_error(writer, id, ERR_INVALID_PARAMS, "invalid textDocument");
                };
                let syms = self.analyzer.document_symbols(&path);
                send_response(writer, id, JsonValue::Array(syms))?;
            }
            "workspace/symbol" => {
                let query = params.get_str("query").unwrap_or("");
                let syms = self.analyzer.workspace_symbols(query);
                send_response(writer, id, JsonValue::Array(syms))?;
            }
            "textDocument/prepareRename" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let prep = self
                    .analyzer
                    .prepare_rename(&path, pos)
                    .unwrap_or(JsonValue::Null);
                send_response(writer, id, prep)?;
            }
            "textDocument/rename" => {
                let Some((path, pos)) = extract_doc_pos(params) else {
                    return send_error(
                        writer,
                        id,
                        ERR_INVALID_PARAMS,
                        "invalid textDocumentPosition",
                    );
                };
                let Some(new_name) = params.get_str("newName") else {
                    return send_error(writer, id, ERR_INVALID_PARAMS, "missing newName");
                };
                match self.analyzer.rename(&path, pos, new_name) {
                    Ok(edit) => send_response(writer, id, edit)?,
                    Err(msg) => send_error(writer, id, ERR_INVALID_PARAMS, &msg)?,
                }
            }
            "textDocument/foldingRange" => {
                let Some(path) = extract_doc_path(params) else {
                    return send_error(writer, id, ERR_INVALID_PARAMS, "invalid textDocument");
                };
                let folds = self.analyzer.folding_ranges(&path);
                send_response(writer, id, JsonValue::Array(folds))?;
            }
            "textDocument/semanticTokens/full" => {
                let Some(path) = extract_doc_path(params) else {
                    return send_error(writer, id, ERR_INVALID_PARAMS, "invalid textDocument");
                };
                let raw = self.analyzer.semantic_tokens(&path);
                let arr: Vec<JsonValue> = raw.into_iter().map(JsonValue::from).collect();
                send_response(
                    writer,
                    id,
                    JsonValue::object([("data", JsonValue::Array(arr))]),
                )?;
            }
            "textDocument/formatting" => {
                if !self.format_enabled {
                    return send_response(writer, id, JsonValue::Array(Vec::new()));
                }
                let Some(path) = extract_doc_path(params) else {
                    return send_error(writer, id, ERR_INVALID_PARAMS, "invalid textDocument");
                };
                match self.analyzer.format_document(&path) {
                    Ok(edits) => send_response(writer, id, JsonValue::Array(edits))?,
                    // If the document currently has a syntax error, gracefully return empty edits!
                    Err(_) => send_response(writer, id, JsonValue::Array(Vec::new()))?,
                }
            }
            _ => {
                send_error(
                    writer,
                    id,
                    ERR_METHOD_NOT_FOUND,
                    &format!("unsupported method `{method}`"),
                )?;
            }
        }
        Ok(())
    }

    fn apply_initialize_params(&mut self, params: &JsonValue) {
        let mut roots = Vec::new();
        if let Some(folders) = params.get_array("workspaceFolders") {
            for f in folders {
                if let Some(uri) = f.get_str("uri")
                    && let Some(p) = uri_to_path(uri)
                {
                    roots.push(p);
                }
            }
        }
        if roots.is_empty() {
            if let Some(uri) = params.get_str("rootUri")
                && let Some(p) = uri_to_path(uri)
            {
                roots.push(p);
            } else if let Some(rp) = params.get_str("rootPath") {
                roots.push(PathBuf::from(rp));
            }
        }
        if !roots.is_empty() {
            self.analyzer.set_workspace_roots(roots);
        }

        if let Some(init_opts) = params.get("initializationOptions") {
            if let Some(sr) = init_opts.get_str("sysroot") {
                let trimmed = sr.trim();
                if !trimmed.is_empty() {
                    self.analyzer
                        .set_sysroot_override(Some(PathBuf::from(trimmed)));
                }
            }
            if let Some(fmt_en) = init_opts.get_bool("formatEnable") {
                self.format_enabled = fmt_en;
            }
        }
    }

    fn publish_package_diagnostics<W: Write>(
        &mut self,
        file: &PathBuf,
        writer: &mut W,
    ) -> io::Result<()> {
        let pkg_root = self.analyzer.find_package_root(file);
        let snap = self.analyzer.ensure_snapshot(file).clone();

        let mut targets: HashSet<PathBuf> = snap.diagnostics.keys().cloned().collect();
        for prev in &self.published_files {
            if prev.starts_with(&pkg_root) {
                targets.insert(prev.clone());
            }
        }
        targets.insert(file.clone());

        let mut sorted: Vec<PathBuf> = targets.into_iter().collect();
        sorted.sort();

        for target in sorted {
            let diags = snap
                .diagnostics
                .get(&target)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if !diags.is_empty() {
                self.published_files.insert(target.clone());
            } else {
                self.published_files.remove(&target);
            }
            self.publish_file_diagnostics(&target, diags, writer)?;
        }
        Ok(())
    }

    fn republish_open_diagnostics<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        let open_paths: Vec<PathBuf> = self.analyzer.open_docs.keys().cloned().collect();
        for p in open_paths {
            self.publish_package_diagnostics(&p, writer)?;
        }
        Ok(())
    }

    fn publish_file_diagnostics<W: Write>(
        &self,
        file: &PathBuf,
        diags: &[LspDiagnostic],
        writer: &mut W,
    ) -> io::Result<()> {
        let uri = path_to_uri(file);
        let items: Vec<JsonValue> = diags.iter().map(LspDiagnostic::to_json).collect();
        let mut fields = vec![
            ("uri", JsonValue::from(uri)),
            ("diagnostics", JsonValue::Array(items)),
        ];
        if let Some(doc) = self.analyzer.open_docs.get(file) {
            fields.push(("version", JsonValue::from(doc.version)));
        }
        send_notification(
            writer,
            "textDocument/publishDiagnostics",
            JsonValue::object(fields),
        )
    }
}

fn extract_doc_path(params: &JsonValue) -> Option<PathBuf> {
    let uri = params.get("textDocument")?.get_str("uri")?;
    uri_to_path(uri)
}

fn extract_doc_pos(params: &JsonValue) -> Option<(PathBuf, LspPosition)> {
    let path = extract_doc_path(params)?;
    let pos = LspPosition::from_json(params.get("position")?)?;
    Some((path, pos))
}

fn build_initialize_result() -> JsonValue {
    let token_types: Vec<JsonValue> = semantic_token_type::LEGEND
        .iter()
        .map(|&s| JsonValue::from(s))
        .collect();
    let token_mods: Vec<JsonValue> = semantic_token_modifier::LEGEND
        .iter()
        .map(|&s| JsonValue::from(s))
        .collect();

    let capabilities = JsonValue::object([
        // TextDocumentSyncKind.Incremental = 2
        (
            "textDocumentSync",
            JsonValue::object([
                ("openClose", JsonValue::Bool(true)),
                ("change", JsonValue::from(2u32)),
                (
                    "save",
                    JsonValue::object([("includeText", JsonValue::Bool(false))]),
                ),
            ]),
        ),
        ("definitionProvider", JsonValue::Bool(true)),
        ("declarationProvider", JsonValue::Bool(true)),
        ("referencesProvider", JsonValue::Bool(true)),
        ("documentHighlightProvider", JsonValue::Bool(true)),
        ("hoverProvider", JsonValue::Bool(true)),
        (
            "completionProvider",
            JsonValue::object([
                ("resolveProvider", JsonValue::Bool(false)),
                (
                    "triggerCharacters",
                    JsonValue::Array(vec![
                        JsonValue::from("."),
                        JsonValue::from("'"),
                        JsonValue::from("%"),
                    ]),
                ),
            ]),
        ),
        (
            "signatureHelpProvider",
            JsonValue::object([(
                "triggerCharacters",
                JsonValue::Array(vec![JsonValue::from("("), JsonValue::from(",")]),
            )]),
        ),
        ("documentSymbolProvider", JsonValue::Bool(true)),
        ("workspaceSymbolProvider", JsonValue::Bool(true)),
        (
            "renameProvider",
            JsonValue::object([("prepareProvider", JsonValue::Bool(true))]),
        ),
        ("documentFormattingProvider", JsonValue::Bool(true)),
        ("foldingRangeProvider", JsonValue::Bool(true)),
        (
            "semanticTokensProvider",
            JsonValue::object([
                (
                    "legend",
                    JsonValue::object([
                        ("tokenTypes", JsonValue::Array(token_types)),
                        ("tokenModifiers", JsonValue::Array(token_mods)),
                    ]),
                ),
                ("full", JsonValue::Bool(true)),
            ]),
        ),
        (
            "workspace",
            JsonValue::object([(
                "workspaceFolders",
                JsonValue::object([
                    ("supported", JsonValue::Bool(true)),
                    ("changeNotifications", JsonValue::Bool(true)),
                ]),
            )]),
        ),
    ]);

    JsonValue::object([
        ("capabilities", capabilities),
        (
            "serverInfo",
            JsonValue::object([
                ("name", JsonValue::from("anar")),
                ("version", JsonValue::from(ANAR_VERSION)),
            ]),
        ),
    ])
}
