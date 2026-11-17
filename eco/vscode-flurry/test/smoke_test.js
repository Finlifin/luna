#!/usr/bin/env node
"use strict";

const assert = require("assert");
const fs = require("fs");
const Module = require("module");
const os = require("os");
const path = require("path");
const { pathToFileURL } = require("url");

const EXT_ROOT = path.resolve(__dirname, "..");
const REPO_ROOT = path.resolve(EXT_ROOT, "..", "..");

function readJson(relPath) {
  const full = path.join(EXT_ROOT, relPath);
  const text = fs.readFileSync(full, "utf8");
  return JSON.parse(text);
}

function verifyStaticAssets() {
  const pkg = readJson("package.json");
  assert.strictEqual(pkg.name, "vscode-flurry");
  assert.strictEqual(pkg.main, "./src/extension.js");
  assert.ok(Array.isArray(pkg.contributes.languages));
  assert.strictEqual(pkg.contributes.languages[0].id, "flurry");
  assert.ok(pkg.contributes.languages[0].extensions.includes(".fl"));

  const langCfg = readJson("language-configuration.json");
  assert.strictEqual(langCfg.comments.lineComment, "--");
  assert.deepStrictEqual(langCfg.comments.blockComment, ["{-", "-}"]);
  new RegExp(langCfg.wordPattern);
  new RegExp(langCfg.folding.markers.start);
  new RegExp(langCfg.folding.markers.end);
  new RegExp(langCfg.indentationRules.increaseIndentPattern);
  new RegExp(langCfg.indentationRules.decreaseIndentPattern);

  const tm = readJson("syntaxes/flurry.tmLanguage.json");
  assert.strictEqual(tm.scopeName, "source.flurry");
  assert.ok(tm.repository && typeof tm.repository === "object");

  const snippets = readJson("snippets/flurry.code-snippets");
  const snippetKeys = Object.keys(snippets);
  assert.ok(snippetKeys.length >= 10, "expected >= 10 Flurry snippets");
  for (const [key, entry] of Object.entries(snippets)) {
    const bodyText = Array.isArray(entry.body) ? entry.body.join("\n") : String(entry.body);
    assert.ok(
      !bodyText.includes("::"),
      `snippet "${key}" must use dot-separated Flurry paths, never '::'`
    );
    assert.ok(
      !/\b(?:Option|Result)\b/.test(bodyText),
      `snippet "${key}" must not reference non-existent Option/Result types`
    );
  }
}

async function testChunkedMockJsonRpc() {
  const { LspClient } = require(path.join(EXT_ROOT, "src", "lsp_client.js"));

  const mockServerScript = `
    let buf = Buffer.alloc(0);
    function send(obj) {
      const body = Buffer.from(JSON.stringify(obj), "utf8");
      const hdr = Buffer.from("Content-Length: " + body.length + "\\r\\n\\r\\n", "ascii");
      const full = Buffer.concat([hdr, body]);
      // Deliberately split frame in half to test client reassembly!
      const mid = Math.floor(full.length / 2);
      process.stdout.write(full.subarray(0, mid));
      setTimeout(() => process.stdout.write(full.subarray(mid)), 5);
    }
    process.stdin.on("data", (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      while (true) {
        const idx = buf.indexOf("\\r\\n\\r\\n");
        if (idx === -1) return;
        const hdr = buf.subarray(0, idx).toString("ascii");
        const m = /content-length:\\s*(\\d+)/i.exec(hdr);
        const len = Number(m[1]);
        if (buf.length < idx + 4 + len) return;
        const msg = JSON.parse(buf.subarray(idx + 4, idx + 4 + len).toString("utf8"));
        buf = buf.subarray(idx + 4 + len);
        if (msg.method === "initialize") {
          send({ jsonrpc: "2.0", id: msg.id, result: { capabilities: { hoverProvider: true }, serverInfo: { name: "mock-anar", version: "0.1.0" } } });
        } else if (msg.method === "initialized") {
          send({ jsonrpc: "2.0", method: "textDocument/publishDiagnostics", params: { uri: "file:///tmp/demo.fl", diagnostics: [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } }, severity: 1, message: "demo utf8 诊断 🚀" }] } });
        } else if (msg.method === "textDocument/hover") {
          send({ jsonrpc: "2.0", id: msg.id, result: { contents: { kind: "markdown", value: "fn demo() -> i32" } } });
        } else if (msg.method === "shutdown") {
          send({ jsonrpc: "2.0", id: msg.id, result: null });
        } else if (msg.method === "exit") {
          process.exit(0);
        }
      }
    });
  `;

  const client = new LspClient({
    command: process.execPath,
    args: ["-e", mockServerScript],
    logger: (line) => console.error(line),
  });

  const diagPromise = new Promise((resolve) => {
    client.once("notification:textDocument/publishDiagnostics", resolve);
  });

  const initRes = await client.initialize({ processId: process.pid, rootUri: null, capabilities: {} });
  assert.strictEqual(initRes.serverInfo.name, "mock-anar");

  const diag = await diagPromise;
  assert.strictEqual(diag.diagnostics.length, 1);
  assert.ok(diag.diagnostics[0].message.includes("诊断 🚀"));

  const hover = await client.sendRequest("textDocument/hover", {
    textDocument: { uri: "file:///tmp/demo.fl" },
    position: { line: 0, character: 1 },
  });
  assert.strictEqual(hover.contents.value, "fn demo() -> i32");

  await client.stop();
}

function testExtensionModuleLoadsWithStubVscode() {
  const origLoad = Module._load;
  Module._load = function patchedLoad(request, parent, isMain) {
    if (request === "vscode") {
      return {
        StatusBarAlignment: { Left: 1, Right: 2 },
        DiagnosticSeverity: { Error: 0, Warning: 1, Information: 2, Hint: 3 },
        DocumentHighlightKind: { Text: 0, Read: 1, Write: 2 },
        FoldingRangeKind: { Comment: 1, Imports: 2, Region: 3 },
        CompletionItemKind: { Text: 0 },
        SymbolKind: { File: 0, Variable: 12 },
        workspace: {
          workspaceFolders: [{ uri: { fsPath: REPO_ROOT, toString: () => pathToFileURL(REPO_ROOT).toString() }, name: "luna" }],
          getConfiguration: () => ({ get: (_k, def) => def }),
        },
      };
    }
    return origLoad.call(this, request, parent, isMain);
  };

  try {
    const ext = require(path.join(EXT_ROOT, "src", "extension.js"));
    assert.strictEqual(typeof ext.activate, "function");
    assert.strictEqual(typeof ext.deactivate, "function");
    assert.deepStrictEqual(ext.serverArguments("/tmp/library path", ["--trace", "", null]),
      ["lsp", "--stdio", "--sysroot", "/tmp/library path", "--trace"]);
    const sysroot = ext.resolveSysroot(EXT_ROOT, "");
    assert.strictEqual(sysroot, path.join(REPO_ROOT, "library"));
  } finally {
    Module._load = origLoad;
  }
}

function findRealLunaBinary() {
  if (process.env.LUNA_BIN) {
    const explicit = path.resolve(process.env.LUNA_BIN);
    fs.accessSync(explicit, fs.constants.X_OK);
    return explicit;
  }
  const candidates = [
    path.join(REPO_ROOT, "target", "debug", "luna"),
    path.join(REPO_ROOT, "target", "release", "luna"),
    "/tmp/luna-dev-002/target/debug/luna",
    "/tmp/luna-dev-002/target/release/luna",
  ].filter(Boolean);

  for (const candidate of candidates) {
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch (_) {
      // continue
    }
  }
  return null;
}

async function testLiveLunaRoundtripIfAvailable() {
  const lunaBin = findRealLunaBinary();
  if (!lunaBin) {
    console.log("[smoke_test] real `luna` binary not found in target cache; skipping live binary check");
    return;
  }

  console.log(`[smoke_test] testing live luna lsp: ${lunaBin}`);
  const { LspClient } = require(path.join(EXT_ROOT, "src", "lsp_client.js"));
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "vscode-flurry-smoke-"));
  const filePath = path.join(tmpDir, "main.fl");
  const unformatted = "--- Compute square.\npub fn   square(x: i32) -> i32 {\n    x * x\n}\n";
  fs.writeFileSync(filePath, unformatted, "utf8");
  const fileUri = pathToFileURL(filePath).toString();

  const client = new LspClient({
    command: lunaBin,
    args: require(path.join(EXT_ROOT, "src", "extension.js")).serverArguments(path.join(REPO_ROOT, "library"), []),
    cwd: tmpDir,
    logger: (line) => console.error(line),
  });

  try {
    const init = await client.initialize({
      processId: process.pid,
      rootUri: pathToFileURL(tmpDir).toString(),
      capabilities: {},
    });
    assert.strictEqual(init.serverInfo.name, "anar");

    const diagPromise = new Promise((resolve) => {
      client.once("notification:textDocument/publishDiagnostics", resolve);
    });

    client.sendNotification("textDocument/didOpen", {
      textDocument: {
        uri: fileUri,
        languageId: "flurry",
        version: 1,
        text: unformatted,
      },
    });

    const diag = await diagPromise;
    assert.strictEqual(diag.diagnostics.length, 0);

    const symbols = await client.sendRequest("textDocument/documentSymbol", {
      textDocument: { uri: fileUri },
    });
    assert.ok(Array.isArray(symbols) && symbols.some((s) => s.name === "square"));

    const edits = await client.sendRequest("textDocument/formatting", {
      textDocument: { uri: fileUri },
      options: { tabSize: 4, insertSpaces: true },
    });
    assert.ok(Array.isArray(edits) && edits.length === 1);
    assert.ok(edits[0].newText.includes("pub fn square(x: i32) -> i32"));

    console.log(`[smoke_test] verified live LspClient <-> luna lsp (${lunaBin}) round-trip`);
  } finally {
    await client.stop();
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

async function main() {
  verifyStaticAssets();
  testExtensionModuleLoadsWithStubVscode();
  await testChunkedMockJsonRpc();
  await testLiveLunaRoundtripIfAvailable();
  console.log("[smoke_test] all checks passed");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
