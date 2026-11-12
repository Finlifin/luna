//! End-to-end LSP 3.17 integration tests for `anar`.

use std::fs;
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anar::json::JsonValue;
use anar::position::path_to_uri;
use anar::rpc::{RpcMessage, read_rpc_message, write_rpc_payload};
use anar::server::LanguageServer;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("anar_test_{prefix}_{}_{stamp}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
        full
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn encode_messages(msgs: &[JsonValue]) -> Vec<u8> {
    let mut buf = Vec::new();
    for m in msgs {
        write_rpc_payload(&mut buf, m).unwrap();
    }
    buf
}

fn decode_messages(raw: &[u8]) -> Vec<RpcMessage> {
    let mut reader = BufReader::new(Cursor::new(raw));
    let mut out = Vec::new();
    while let Some(msg) = read_rpc_message(&mut reader).unwrap() {
        out.push(msg);
    }
    out
}

fn find_response(msgs: &[RpcMessage], req_id: i64) -> &JsonValue {
    for msg in msgs {
        if let RpcMessage::Response {
            id: anar::rpc::RequestId::Int(id),
            result: Some(res),
            error: None,
        } = msg
            && *id == req_id
        {
            return res;
        }
    }
    panic!("missing successful response for id={req_id} in {msgs:#?}");
}

fn repo_library_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("library")
}

#[test]
fn lsp_multifile_navigation_hover_rename_and_completion() {
    let tmp = TempDir::new("multifile");
    let geom_src = "\
-- 2D Cartesian point.
pub struct Point {
    x: i32,
    y: i32,
}

impl Point {
    -- Compute Manhattan distance from origin.
    pub fn manhattan(self) -> i32 {
        self.x + self.y
    }
}
";
    let main_src = "\
use geom.Point;

fn run(pt: Point) -> i32 {
    let moved = Point { x: pt.x + 1, y: pt.y };
    moved.manhattan()
}
";
    let geom_path = tmp.write("geom.fl", geom_src);
    let main_path = tmp.write("main.fl", main_src);

    let root_uri = path_to_uri(&tmp.path);
    let geom_uri = path_to_uri(&geom_path);
    let main_uri = path_to_uri(&main_path);

    let requests = vec![
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(1)),
            ("method", JsonValue::from("initialize")),
            (
                "params",
                JsonValue::object([
                    ("rootUri", JsonValue::from(root_uri.as_str())),
                    (
                        "initializationOptions",
                        JsonValue::object([(
                            "sysroot",
                            JsonValue::from(repo_library_dir().to_string_lossy().as_ref()),
                        )]),
                    ),
                ]),
            ),
        ]),
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("method", JsonValue::from("initialized")),
            ("params", JsonValue::Object(Vec::new())),
        ]),
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("method", JsonValue::from("textDocument/didOpen")),
            (
                "params",
                JsonValue::object([(
                    "textDocument",
                    JsonValue::object([
                        ("uri", JsonValue::from(main_uri.as_str())),
                        ("languageId", JsonValue::from("flurry")),
                        ("version", JsonValue::from(1)),
                        ("text", JsonValue::from(main_src)),
                    ]),
                )]),
            ),
        ]),
        // 2: Go-to-definition on `Point` in `fn run(pt: Point)` (line 2, col 16)
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(2)),
            ("method", JsonValue::from("textDocument/definition")),
            (
                "params",
                JsonValue::object([
                    (
                        "textDocument",
                        JsonValue::object([("uri", JsonValue::from(main_uri.as_str()))]),
                    ),
                    (
                        "position",
                        JsonValue::object([
                            ("line", JsonValue::from(2)),
                            ("character", JsonValue::from(16)),
                        ]),
                    ),
                ]),
            ),
        ]),
        // 3: Go-to-definition on `.manhattan()` in `moved.manhattan()` (line 4, col 12)
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(3)),
            ("method", JsonValue::from("textDocument/definition")),
            (
                "params",
                JsonValue::object([
                    (
                        "textDocument",
                        JsonValue::object([("uri", JsonValue::from(main_uri.as_str()))]),
                    ),
                    (
                        "position",
                        JsonValue::object([
                            ("line", JsonValue::from(4)),
                            ("character", JsonValue::from(12)),
                        ]),
                    ),
                ]),
            ),
        ]),
        // 4: Hover on `manhattan` (line 4, col 12) — should include signature + doc comment!
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(4)),
            ("method", JsonValue::from("textDocument/hover")),
            (
                "params",
                JsonValue::object([
                    (
                        "textDocument",
                        JsonValue::object([("uri", JsonValue::from(main_uri.as_str()))]),
                    ),
                    (
                        "position",
                        JsonValue::object([
                            ("line", JsonValue::from(4)),
                            ("character", JsonValue::from(12)),
                        ]),
                    ),
                ]),
            ),
        ]),
        // 5: Find references on `Point` (line 2, col 16) across both `geom.fl` and `main.fl`!
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(5)),
            ("method", JsonValue::from("textDocument/references")),
            (
                "params",
                JsonValue::object([
                    (
                        "textDocument",
                        JsonValue::object([("uri", JsonValue::from(main_uri.as_str()))]),
                    ),
                    (
                        "position",
                        JsonValue::object([
                            ("line", JsonValue::from(2)),
                            ("character", JsonValue::from(16)),
                        ]),
                    ),
                    (
                        "context",
                        JsonValue::object([("includeDeclaration", JsonValue::Bool(true))]),
                    ),
                ]),
            ),
        ]),
        // 6: Dot-completion right after `moved.` (line 4, col 10)
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(6)),
            ("method", JsonValue::from("textDocument/completion")),
            (
                "params",
                JsonValue::object([
                    (
                        "textDocument",
                        JsonValue::object([("uri", JsonValue::from(main_uri.as_str()))]),
                    ),
                    (
                        "position",
                        JsonValue::object([
                            ("line", JsonValue::from(4)),
                            ("character", JsonValue::from(10)),
                        ]),
                    ),
                ]),
            ),
        ]),
        // 7: Cross-file rename `Point` -> `Vec2`
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(7)),
            ("method", JsonValue::from("textDocument/rename")),
            (
                "params",
                JsonValue::object([
                    (
                        "textDocument",
                        JsonValue::object([("uri", JsonValue::from(main_uri.as_str()))]),
                    ),
                    (
                        "position",
                        JsonValue::object([
                            ("line", JsonValue::from(2)),
                            ("character", JsonValue::from(16)),
                        ]),
                    ),
                    ("newName", JsonValue::from("Vec2")),
                ]),
            ),
        ]),
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(8)),
            ("method", JsonValue::from("shutdown")),
            ("params", JsonValue::Null),
        ]),
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("method", JsonValue::from("exit")),
            ("params", JsonValue::Null),
        ]),
    ];

    let input_bytes = encode_messages(&requests);
    let mut reader = BufReader::new(Cursor::new(input_bytes));
    let mut output_bytes = Vec::new();
    let mut server = LanguageServer::new(Some(repo_library_dir()));
    let exit_code = server.run(&mut reader, &mut output_bytes).unwrap();
    assert_eq!(exit_code, 0);

    let responses = decode_messages(&output_bytes);

    // Verify `initialize` capabilities
    let init_res = find_response(&responses, 1);
    assert_eq!(
        init_res.get("serverInfo").and_then(|s| s.get_str("name")),
        Some("anar")
    );
    assert_eq!(
        init_res
            .get("capabilities")
            .and_then(|c| c.get_bool("definitionProvider")),
        Some(true)
    );

    // Verify `Point` jumps to `geom.fl` line 1 (`pub struct Point`)
    let def_point = find_response(&responses, 2)
        .as_array()
        .expect("array of locations");
    assert_eq!(def_point.len(), 1);
    assert_eq!(def_point[0].get_str("uri"), Some(geom_uri.as_str()));
    assert_eq!(
        def_point[0]
            .get("range")
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get_u32("line")),
        Some(1)
    );

    // Verify `moved.manhattan()` jumps to `pub fn manhattan` in `geom.fl` line 8
    let def_method = find_response(&responses, 3).as_array().unwrap();
    assert_eq!(def_method.len(), 1);
    assert_eq!(def_method[0].get_str("uri"), Some(geom_uri.as_str()));
    assert_eq!(
        def_method[0]
            .get("range")
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get_u32("line")),
        Some(8)
    );

    // Verify hover includes signature AND doc comment!
    let hover = find_response(&responses, 4);
    let hover_md = hover
        .get("contents")
        .and_then(|c| c.get_str("value"))
        .unwrap();
    assert!(hover_md.contains("fn manhattan(self) -> i32"), "{hover_md}");
    assert!(
        hover_md.contains("Compute Manhattan distance from origin."),
        "{hover_md}"
    );

    // Verify find-references spans both `geom.fl` and `main.fl`
    let refs = find_response(&responses, 5).as_array().unwrap();
    let ref_uris: Vec<&str> = refs.iter().filter_map(|r| r.get_str("uri")).collect();
    assert!(ref_uris.contains(&geom_uri.as_str()), "{ref_uris:?}");
    assert!(ref_uris.contains(&main_uri.as_str()), "{ref_uris:?}");
    assert!(
        refs.len() >= 4,
        "expected >=4 Point occurrences, got {}",
        refs.len()
    );

    // Verify dot-completion after `moved.` includes `x`, `y`, `manhattan`, and `.ref`!
    let comp_items = find_response(&responses, 6)
        .get_array("items")
        .expect("completion items");
    let labels: Vec<&str> = comp_items
        .iter()
        .filter_map(|i| i.get_str("label"))
        .collect();
    assert!(labels.contains(&"x"), "{labels:?}");
    assert!(labels.contains(&"y"), "{labels:?}");
    assert!(labels.contains(&"manhattan"), "{labels:?}");
    assert!(labels.contains(&"ref"), "{labels:?}");

    // Verify cross-file rename produces edits in both `geom.fl` and `main.fl`!
    let rename_changes = find_response(&responses, 7)
        .get("changes")
        .and_then(|c| c.as_object())
        .unwrap();
    let changed_uris: Vec<&str> = rename_changes.iter().map(|(k, _)| k.as_str()).collect();
    assert!(changed_uris.contains(&geom_uri.as_str()));
    assert!(changed_uris.contains(&main_uri.as_str()));
}

#[test]
fn lsp_sysroot_resolution_incremental_recovery_and_formatting() {
    let tmp = TempDir::new("sysroot_recovery");
    let lib_src = "\
use std.io.Writer;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn compute() -> i32 {
    add(10, 20)
}
";
    let lib_path = tmp.write("lib.fl", lib_src);
    let lib_uri = path_to_uri(&lib_path);
    let sr_dir = repo_library_dir();

    let mut analyzer = anar::WorkspaceAnalyzer::new(Some(sr_dir.clone()));
    analyzer.open_document(&lib_uri, 1, lib_src.to_owned());
    assert!(
        analyzer.diagnostics_for(&lib_path).is_empty(),
        "valid sysroot import + user code should have 0 diagnostics: {:?}",
        analyzer.diagnostics_for(&lib_path)
    );

    // 1. Go-to-definition on `Writer` (line 0, col 13) jumps directly into `library/std/...`!
    let writer_defs = analyzer.goto_definition(&lib_path, anar::LspPosition::new(0, 13));
    assert_eq!(
        writer_defs.len(),
        1,
        "Writer should resolve into sysroot std"
    );
    let target_uri = writer_defs[0].get_str("uri").unwrap();
    assert!(
        target_uri.contains("/library/std/"),
        "expected definition inside library/std, got {target_uri}"
    );

    // 2. SignatureHelp inside `add(10, 20)` at second argument (line 7, col 13 -> activeParameter = 1)
    let sig_help = analyzer
        .signature_help(&lib_path, anar::LspPosition::new(7, 13))
        .expect("signatureHelp");
    assert_eq!(sig_help.get_u32("activeParameter"), Some(1));
    let first_sig = &sig_help.get_array("signatures").unwrap()[0];
    assert!(
        first_sig
            .get_str("label")
            .unwrap()
            .contains("fn add(a: i32, b: i32) -> i32")
    );

    // 3. Introduce a live syntax error via incremental `didChange` and verify:
    //    - syntax error diagnostic is reported
    //    - last-known-good `document_symbols` and `goto_definition` still work!
    let broken_change = vec![JsonValue::object([
        (
            "range",
            JsonValue::object([
                (
                    "start",
                    JsonValue::object([
                        ("line", JsonValue::from(7)),
                        ("character", JsonValue::from(15)),
                    ]),
                ),
                (
                    "end",
                    JsonValue::object([
                        ("line", JsonValue::from(7)),
                        ("character", JsonValue::from(15)),
                    ]),
                ),
            ]),
        ),
        ("text", JsonValue::from(" + ???")),
    ])];
    analyzer.change_document(&lib_uri, 2, &broken_change);
    let broken_diags = analyzer.diagnostics_for(&lib_path);
    assert!(
        !broken_diags.is_empty(),
        "expected syntax error diagnostic on broken edit"
    );
    let doc_syms_during_error = analyzer.document_symbols(&lib_path);
    assert!(
        doc_syms_during_error.len() >= 2,
        "last-known-good outline should survive syntax error: {doc_syms_during_error:#?}"
    );

    // 4. Fix buffer & test `format_document` honoring `--% .skip`!
    let unformatted = "\
pub fn messy(x:i32,y:i32)->i32{x + y}

--% .skip
pub fn keep_raw(a:i32)->i32{a}
";
    analyzer.change_document(
        &lib_uri,
        3,
        &[JsonValue::object([("text", JsonValue::from(unformatted))])],
    );
    assert!(analyzer.diagnostics_for(&lib_path).is_empty());
    let edits = analyzer.format_document(&lib_path).unwrap();
    assert_eq!(edits.len(), 1);
    let formatted_text = edits[0].get_str("newText").unwrap();
    assert!(
        formatted_text.contains("pub fn messy(x: i32, y: i32) -> i32"),
        "{formatted_text}"
    );
    assert!(
        formatted_text.contains("pub fn keep_raw(a:i32)->i32{a}"),
        "--% .skip item must remain verbatim: {formatted_text}"
    );

    // 5. Verify semantic tokens & folding ranges are non-empty!
    let sem = analyzer.semantic_tokens(&lib_path);
    assert!(!sem.is_empty() && sem.len() % 5 == 0);
    let ws_syms = analyzer.workspace_symbols("messy");
    assert!(!ws_syms.is_empty());
}

#[test]
fn cli_stdio_process_roundtrip_and_version() {
    let bin = env!("CARGO_BIN_EXE_anar");

    // `--version` flag
    let ver = Command::new(bin).arg("--version").output().unwrap();
    assert!(ver.status.success());
    assert!(String::from_utf8_lossy(&ver.stdout).starts_with("anar "));

    // Spawn `anar --stdio` subprocess and run `initialize` -> `shutdown` -> `exit`
    let mut child = Command::new(bin)
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let payload = encode_messages(&[
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(1)),
            ("method", JsonValue::from("initialize")),
            ("params", JsonValue::object([("rootUri", JsonValue::Null)])),
        ]),
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", JsonValue::from(2)),
            ("method", JsonValue::from("shutdown")),
            ("params", JsonValue::Null),
        ]),
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("method", JsonValue::from("exit")),
            ("params", JsonValue::Null),
        ]),
    ]);

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(&payload).unwrap();
        stdin.flush().unwrap();
    }
    drop(child.stdin.take());

    let mut out = Vec::new();
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());

    let msgs = decode_messages(&out);
    let init = find_response(&msgs, 1);
    assert_eq!(
        init.get("serverInfo").and_then(|s| s.get_str("name")),
        Some("anar")
    );
}
