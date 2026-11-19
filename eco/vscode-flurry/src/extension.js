"use strict";

const fs = require("fs");
const path = require("path");
const vscode = require("vscode");
const { LspClient } = require("./lsp_client");

const LANGUAGE_ID = "flurry";
const DOCUMENT_SELECTOR = [{ scheme: "file", language: LANGUAGE_ID }];

const DEFAULT_TOKEN_TYPES = [
  "namespace",
  "type",
  "struct",
  "enum",
  "interface",
  "typeParameter",
  "parameter",
  "variable",
  "property",
  "enumMember",
  "function",
  "method",
  "macro",
  "keyword",
  "modifier",
  "comment",
  "string",
  "number",
  "operator",
];

const DEFAULT_TOKEN_MODIFIERS = [
  "declaration",
  "definition",
  "readonly",
  "modification",
  "documentation",
];

/** @type {LspClient | null} */
let activeClient = null;
/** @type {vscode.Disposable[]} */
let serverFeatureDisposables = [];
/** @type {vscode.OutputChannel | null} */
let outputChannel = null;
/** @type {vscode.DiagnosticCollection | null} */
let diagnosticCollection = null;
/** @type {vscode.StatusBarItem | null} */
let statusBarItem = null;

/**
 * @param {vscode.ExtensionContext} context
 */
async function activate(context) {
  outputChannel = vscode.window.createOutputChannel("Flurry Language Server (anar)");
  diagnosticCollection = vscode.languages.createDiagnosticCollection(LANGUAGE_ID);
  statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 10);
  statusBarItem.command = "flurry.showOutputChannel";
  statusBarItem.text = "$(sync~spin) Flurry: starting";
  statusBarItem.tooltip = "Flurry Language Server (anar)";
  statusBarItem.show();

  context.subscriptions.push(outputChannel, diagnosticCollection, statusBarItem);

  context.subscriptions.push(
    vscode.commands.registerCommand("flurry.showOutputChannel", () => {
      if (outputChannel) {
        outputChannel.show(true);
      }
    }),
    vscode.commands.registerCommand("flurry.restartServer", async () => {
      await restartLanguageServer(context, true);
    })
  );

  // Sync open/change/save/close events for `.fl` documents.
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => {
      if (isFlurryDocument(doc) && activeClient && activeClient.isRunning()) {
        activeClient.sendNotification("textDocument/didOpen", {
          textDocument: {
            uri: doc.uri.toString(),
            languageId: LANGUAGE_ID,
            version: doc.version,
            text: doc.getText(),
          },
        });
      }
    }),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (!isFlurryDocument(event.document) || !activeClient || !activeClient.isRunning()) {
        return;
      }
      if (event.contentChanges.length === 0) {
        return;
      }
      const contentChanges = event.contentChanges.map((change) => ({
        range: toLspRange(change.range),
        rangeLength: change.rangeLength,
        text: change.text,
      }));
      activeClient.sendNotification("textDocument/didChange", {
        textDocument: {
          uri: event.document.uri.toString(),
          version: event.document.version,
        },
        contentChanges,
      });
    }),
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (isFlurryDocument(doc) && activeClient && activeClient.isRunning()) {
        activeClient.sendNotification("textDocument/didSave", {
          textDocument: { uri: doc.uri.toString() },
        });
      }
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      if (!isFlurryDocument(doc)) {
        return;
      }
      if (diagnosticCollection) {
        diagnosticCollection.delete(doc.uri);
      }
      if (activeClient && activeClient.isRunning()) {
        activeClient.sendNotification("textDocument/didClose", {
          textDocument: { uri: doc.uri.toString() },
        });
      }
    }),
    vscode.workspace.onDidChangeWorkspaceFolders((event) => {
      if (!activeClient || !activeClient.isRunning()) {
        return;
      }
      activeClient.sendNotification("workspace/didChangeWorkspaceFolders", {
        event: {
          added: event.added.map((f) => ({ uri: f.uri.toString(), name: f.name })),
          removed: event.removed.map((f) => ({ uri: f.uri.toString(), name: f.name })),
        },
      });
    }),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (!event.affectsConfiguration("flurry")) {
        return;
      }
      if (
        event.affectsConfiguration("flurry.anar.enable") ||
        event.affectsConfiguration("flurry.anar.path") ||
        event.affectsConfiguration("flurry.anar.extraArgs")
      ) {
        await restartLanguageServer(context, false);
        return;
      }
      if (activeClient && activeClient.isRunning()) {
        activeClient.sendNotification("workspace/didChangeConfiguration", {
          settings: { flurry: readFlurrySettings() },
        });
      }
    })
  );

  // Watch external `.fl` and manifest changes on disk.
  const watcher = vscode.workspace.createFileSystemWatcher(
    "**/{*.fl,flurry.toml,flurryfmt.fl}"
  );
  const notifyWatched = (uri, type) => {
    if (activeClient && activeClient.isRunning()) {
      activeClient.sendNotification("workspace/didChangeWatchedFiles", {
        changes: [{ uri: uri.toString(), type }],
      });
    }
  };
  watcher.onDidCreate((uri) => notifyWatched(uri, 1));
  watcher.onDidChange((uri) => notifyWatched(uri, 2));
  watcher.onDidDelete((uri) => notifyWatched(uri, 3));
  context.subscriptions.push(watcher);

  await startLanguageServer(context, false);
}

async function deactivate() {
  disposeFeatureProviders();
  if (activeClient) {
    const client = activeClient;
    activeClient = null;
    await client.stop();
  }
}

/**
 * @param {vscode.ExtensionContext} context
 * @param {boolean} notifyUser
 */
async function restartLanguageServer(context, notifyUser) {
  disposeFeatureProviders();
  if (diagnosticCollection) {
    diagnosticCollection.clear();
  }
  if (activeClient) {
    const prev = activeClient;
    activeClient = null;
    await prev.stop();
  }
  await startLanguageServer(context, notifyUser);
}

/**
 * @param {vscode.ExtensionContext} context
 * @param {boolean} notifyUser
 */
async function startLanguageServer(context, notifyUser) {
  const cfg = vscode.workspace.getConfiguration("flurry");
  if (!cfg.get("anar.enable", true)) {
    updateStatusBar("$(circle-slash) Flurry: disabled", "anar language server is disabled");
    return;
  }

  const binaryPath = resolveLunaBinary(context.extensionPath, cfg.get("anar.path", ""));
  if (!binaryPath) {
    const msg =
      "Could not find the `luna` executable. Build luna with `python3 -B development/cargo.py build -p luna` or set `flurry.anar.path` to luna in Settings.";
    logLine(`[warn] ${msg}`);
    updateStatusBar("$(warning) Flurry: luna not found", msg);
    if (notifyUser) {
      vscode.window.showWarningMessage(msg);
    }
    return;
  }

  const sysrootPath = resolveSysroot(context.extensionPath, cfg.get("anar.sysroot", ""));
  const extraArgs = cfg.get("anar.extraArgs", []);
  const args = serverArguments(sysrootPath, extraArgs);

  const workspaceFolders = vscode.workspace.workspaceFolders || [];
  const cwd =
    workspaceFolders.length > 0 ? workspaceFolders[0].uri.fsPath : context.extensionPath;

  logLine(`[info] Starting luna lsp: ${binaryPath} ${args.join(" ")}`);
  updateStatusBar("$(sync~spin) Flurry: starting", `Starting ${binaryPath}`);

  const client = new LspClient({
    command: binaryPath,
    args,
    cwd,
    logger: logLine,
    getTraceLevel: () =>
      vscode.workspace.getConfiguration("flurry").get("trace.server", "off"),
  });

  client.on("notification:textDocument/publishDiagnostics", (params) => {
    handlePublishDiagnostics(params);
  });

  client.on("exit", ({ code, signal, intentional }) => {
    if (!intentional && activeClient === client) {
      const detail = `anar exited unexpectedly (code=${code}, signal=${signal || "none"})`;
      logLine(`[error] ${detail}`);
      updateStatusBar("$(error) Flurry: stopped", detail);
    }
  });

  activeClient = client;

  try {
    const rootUri = workspaceFolders.length > 0 ? workspaceFolders[0].uri.toString() : null;
    const initResult = await client.initialize({
      processId: process.pid,
      rootUri,
      workspaceFolders: workspaceFolders.map((f) => ({
        uri: f.uri.toString(),
        name: f.name,
      })),
      initializationOptions: {
        flurry: readFlurrySettings(sysrootPath),
      },
      capabilities: buildClientCapabilities(),
    });

    if (activeClient !== client) {
      await client.stop();
      return;
    }

    registerFeatureProviders(initResult);

    // Push all currently open `.fl` buffers immediately after initialization.
    for (const doc of vscode.workspace.textDocuments) {
      if (isFlurryDocument(doc)) {
        client.sendNotification("textDocument/didOpen", {
          textDocument: {
            uri: doc.uri.toString(),
            languageId: LANGUAGE_ID,
            version: doc.version,
            text: doc.getText(),
          },
        });
      }
    }

    const version =
      initResult && initResult.serverInfo && initResult.serverInfo.version
        ? ` v${initResult.serverInfo.version}`
        : "";
    updateStatusBar(
      `$(check) Flurry: anar${version}`,
      `anar${version} (${binaryPath})${sysrootPath ? `\nsysroot: ${sysrootPath}` : ""}`
    );
    if (notifyUser) {
      vscode.window.showInformationMessage(`Flurry language server restarted (anar${version}).`);
    }
  } catch (err) {
    logLine(`[error] Failed to initialize anar: ${err.message}`);
    updateStatusBar("$(error) Flurry: init failed", err.message);
    if (activeClient === client) {
      activeClient = null;
    }
    await client.stop();
  }
}

function registerFeatureProviders(initResult) {
  disposeFeatureProviders();

  const caps = (initResult && initResult.capabilities) || {};
  const legendSpec =
    caps.semanticTokensProvider && caps.semanticTokensProvider.legend
      ? caps.semanticTokensProvider.legend
      : { tokenTypes: DEFAULT_TOKEN_TYPES, tokenModifiers: DEFAULT_TOKEN_MODIFIERS };

  const legend = new vscode.SemanticTokensLegend(
    Array.isArray(legendSpec.tokenTypes) ? legendSpec.tokenTypes : DEFAULT_TOKEN_TYPES,
    Array.isArray(legendSpec.tokenModifiers) ? legendSpec.tokenModifiers : DEFAULT_TOKEN_MODIFIERS
  );

  serverFeatureDisposables.push(
    vscode.languages.registerDefinitionProvider(DOCUMENT_SELECTOR, {
      async provideDefinition(doc, pos, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return null;
        }
        const res = await activeClient.sendRequest(
          "textDocument/definition",
          toTextDocumentPosition(doc, pos),
          token
        );
        return fromLspLocations(res);
      },
    }),

    vscode.languages.registerDeclarationProvider(DOCUMENT_SELECTOR, {
      async provideDeclaration(doc, pos, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return null;
        }
        const res = await activeClient.sendRequest(
          "textDocument/declaration",
          toTextDocumentPosition(doc, pos),
          token
        );
        return fromLspLocations(res);
      },
    }),

    vscode.languages.registerReferenceProvider(DOCUMENT_SELECTOR, {
      async provideReferences(doc, pos, context, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return [];
        }
        const res = await activeClient.sendRequest(
          "textDocument/references",
          {
            ...toTextDocumentPosition(doc, pos),
            context: { includeDeclaration: Boolean(context && context.includeDeclaration) },
          },
          token
        );
        return fromLspLocations(res) || [];
      },
    }),

    vscode.languages.registerDocumentHighlightProvider(DOCUMENT_SELECTOR, {
      async provideDocumentHighlights(doc, pos, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return [];
        }
        const res = await activeClient.sendRequest(
          "textDocument/documentHighlight",
          toTextDocumentPosition(doc, pos),
          token
        );
        if (!Array.isArray(res)) {
          return [];
        }
        return res.map((item) => {
          const kind =
            item.kind === 3
              ? vscode.DocumentHighlightKind.Write
              : item.kind === 2
                ? vscode.DocumentHighlightKind.Read
                : vscode.DocumentHighlightKind.Text;
          return new vscode.DocumentHighlight(fromLspRange(item.range), kind);
        });
      },
    }),

    vscode.languages.registerHoverProvider(DOCUMENT_SELECTOR, {
      async provideHover(doc, pos, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return null;
        }
        const res = await activeClient.sendRequest(
          "textDocument/hover",
          toTextDocumentPosition(doc, pos),
          token
        );
        if (!res || !res.contents) {
          return null;
        }
        const md = new vscode.MarkdownString(
          typeof res.contents === "string" ? res.contents : res.contents.value || ""
        );
        md.isTrusted = false;
        return new vscode.Hover(md, res.range ? fromLspRange(res.range) : undefined);
      },
    }),

    vscode.languages.registerCompletionItemProvider(
      DOCUMENT_SELECTOR,
      {
        async provideCompletionItems(doc, pos, token) {
          if (!activeClient || !activeClient.isRunning()) {
            return [];
          }
          const res = await activeClient.sendRequest(
            "textDocument/completion",
            toTextDocumentPosition(doc, pos),
            token
          );
          const items = Array.isArray(res) ? res : res && Array.isArray(res.items) ? res.items : [];
          return items.map(fromLspCompletionItem);
        },
      },
      ".",
      "'",
      "%"
    ),

    vscode.languages.registerSignatureHelpProvider(
      DOCUMENT_SELECTOR,
      {
        async provideSignatureHelp(doc, pos, token) {
          if (!activeClient || !activeClient.isRunning()) {
            return null;
          }
          const res = await activeClient.sendRequest(
            "textDocument/signatureHelp",
            toTextDocumentPosition(doc, pos),
            token
          );
          if (!res || !Array.isArray(res.signatures) || res.signatures.length === 0) {
            return null;
          }
          const help = new vscode.SignatureHelp();
          help.activeSignature =
            typeof res.activeSignature === "number" ? res.activeSignature : 0;
          help.activeParameter =
            typeof res.activeParameter === "number" ? res.activeParameter : 0;
          help.signatures = res.signatures.map((sig) => {
            const info = new vscode.SignatureInformation(
              sig.label,
              sig.documentation ? new vscode.MarkdownString(sig.documentation.value || sig.documentation) : undefined
            );
            if (Array.isArray(sig.parameters)) {
              info.parameters = sig.parameters.map(
                (p) => new vscode.ParameterInformation(p.label, p.documentation)
              );
            }
            return info;
          });
          return help;
        },
      },
      "(",
      ","
    ),

    vscode.languages.registerDocumentSymbolProvider(DOCUMENT_SELECTOR, {
      async provideDocumentSymbols(doc, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return [];
        }
        const res = await activeClient.sendRequest(
          "textDocument/documentSymbol",
          { textDocument: { uri: doc.uri.toString() } },
          token
        );
        if (!Array.isArray(res)) {
          return [];
        }
        return res.map(fromLspDocumentSymbol);
      },
    }),

    vscode.languages.registerWorkspaceSymbolProvider({
      async provideWorkspaceSymbols(query, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return [];
        }
        const res = await activeClient.sendRequest(
          "workspace/symbol",
          { query: query || "" },
          token
        );
        if (!Array.isArray(res)) {
          return [];
        }
        return res.map(
          (sym) =>
            new vscode.SymbolInformation(
              sym.name,
              fromLspSymbolKind(sym.kind),
              sym.containerName || "",
              new vscode.Location(
                vscode.Uri.parse(sym.location.uri),
                fromLspRange(sym.location.range)
              )
            )
        );
      },
    }),

    vscode.languages.registerRenameProvider(DOCUMENT_SELECTOR, {
      async prepareRename(doc, pos, token) {
        if (!activeClient || !activeClient.isRunning()) {
          throw new Error("anar language server is not running");
        }
        const res = await activeClient.sendRequest(
          "textDocument/prepareRename",
          toTextDocumentPosition(doc, pos),
          token
        );
        if (!res) {
          throw new Error("Cannot rename symbol at this position");
        }
        if (res.range) {
          return {
            range: fromLspRange(res.range),
            placeholder: res.placeholder || doc.getText(fromLspRange(res.range)),
          };
        }
        return fromLspRange(res);
      },
      async provideRenameEdits(doc, pos, newName, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return null;
        }
        const res = await activeClient.sendRequest(
          "textDocument/rename",
          {
            ...toTextDocumentPosition(doc, pos),
            newName,
          },
          token
        );
        if (!res || !res.changes) {
          return null;
        }
        const wsEdit = new vscode.WorkspaceEdit();
        for (const [uriStr, edits] of Object.entries(res.changes)) {
          const uri = vscode.Uri.parse(uriStr);
          if (Array.isArray(edits)) {
            for (const edit of edits) {
              wsEdit.replace(uri, fromLspRange(edit.range), edit.newText);
            }
          }
        }
        return wsEdit;
      },
    }),

    vscode.languages.registerDocumentFormattingEditProvider(DOCUMENT_SELECTOR, {
      async provideDocumentFormattingEdits(doc, options, token) {
        const fmtEnabled = vscode.workspace
          .getConfiguration("flurry")
          .get("format.enable", true);
        if (!fmtEnabled || !activeClient || !activeClient.isRunning()) {
          return [];
        }
        const res = await activeClient.sendRequest(
          "textDocument/formatting",
          {
            textDocument: { uri: doc.uri.toString() },
            options: {
              tabSize: options ? options.tabSize : 4,
              insertSpaces: options ? options.insertSpaces : true,
            },
          },
          token
        );
        if (!Array.isArray(res)) {
          return [];
        }
        return res.map((edit) => vscode.TextEdit.replace(fromLspRange(edit.range), edit.newText));
      },
    }),

    vscode.languages.registerFoldingRangeProvider(DOCUMENT_SELECTOR, {
      async provideFoldingRanges(doc, _context, token) {
        if (!activeClient || !activeClient.isRunning()) {
          return [];
        }
        const res = await activeClient.sendRequest(
          "textDocument/foldingRange",
          { textDocument: { uri: doc.uri.toString() } },
          token
        );
        if (!Array.isArray(res)) {
          return [];
        }
        return res.map((r) => {
          const kind =
            r.kind === "comment"
              ? vscode.FoldingRangeKind.Comment
              : r.kind === "imports"
                ? vscode.FoldingRangeKind.Imports
                : vscode.FoldingRangeKind.Region;
          return new vscode.FoldingRange(r.startLine, r.endLine, kind);
        });
      },
    }),

    vscode.languages.registerDocumentSemanticTokensProvider(
      DOCUMENT_SELECTOR,
      {
        async provideDocumentSemanticTokens(doc, token) {
          if (!activeClient || !activeClient.isRunning()) {
            return null;
          }
          const res = await activeClient.sendRequest(
            "textDocument/semanticTokens/full",
            { textDocument: { uri: doc.uri.toString() } },
            token
          );
          if (!res || !Array.isArray(res.data)) {
            return null;
          }
          return new vscode.SemanticTokens(new Uint32Array(res.data));
        },
      },
      legend
    )
  );
}

function disposeFeatureProviders() {
  for (const d of serverFeatureDisposables) {
    try {
      d.dispose();
    } catch (_) {
      // Ignore disposal error
    }
  }
  serverFeatureDisposables = [];
}

function handlePublishDiagnostics(params) {
  if (!diagnosticCollection || !params || typeof params.uri !== "string") {
    return;
  }
  const uri = vscode.Uri.parse(params.uri);
  const rawList = Array.isArray(params.diagnostics) ? params.diagnostics : [];
  const converted = rawList.map((d) => {
    const severity =
      d.severity === 1
        ? vscode.DiagnosticSeverity.Error
        : d.severity === 2
          ? vscode.DiagnosticSeverity.Warning
          : d.severity === 3
            ? vscode.DiagnosticSeverity.Information
            : vscode.DiagnosticSeverity.Hint;
    const diag = new vscode.Diagnostic(fromLspRange(d.range), d.message || "", severity);
    diag.source = d.source || "anar";
    if (d.code !== undefined) {
      diag.code = d.code;
    }
    return diag;
  });
  diagnosticCollection.set(uri, converted);
}

/** Build the single CLI's LSP subcommand without invoking a shell. */
function serverArguments(sysrootPath, extraArgs) {
  const args = ["lsp", "--stdio"];
  if (sysrootPath) {
    args.push("--sysroot", sysrootPath);
  }
  if (Array.isArray(extraArgs)) {
    for (const arg of extraArgs) {
      if (typeof arg === "string" && arg.length > 0) {
        args.push(arg);
      }
    }
  }
  return args;
}

/**
 * Discover the `luna` executable across explicit setting, workspace builds, dev cache, and PATH.
 * @param {string} extensionPath
 * @param {string} configuredPath
 * @returns {string | null}
 */
function resolveLunaBinary(extensionPath, configuredPath) {
  const exeName = process.platform === "win32" ? "luna.exe" : "luna";
  if (configuredPath && configuredPath.trim().length > 0) {
    const expanded = configuredPath.trim();
    if (isExecutableFile(expanded)) {
      return expanded;
    }
    return null;
  }

  const candidateRoots = [];
  for (const folder of vscode.workspace.workspaceFolders || []) {
    candidateRoots.push(folder.uri.fsPath);
  }
  // Walk up from the extension directory (e.g. `<repo>/eco/vscode-flurry`).
  let dir = extensionPath;
  for (let i = 0; i < 4; i++) {
    candidateRoots.push(dir);
    const parent = path.dirname(dir);
    if (parent === dir) {
      break;
    }
    dir = parent;
  }

  const targetRelPaths = [
    path.join("target", "release", exeName),
    path.join("target", "debug", exeName),
  ];

  for (const root of candidateRoots) {
    for (const rel of targetRelPaths) {
      const full = path.join(root, rel);
      if (isExecutableFile(full)) {
        return full;
      }
    }
  }

  // Check shared Luna dev build cache if present on Linux/macOS.
  for (const sharedTarget of ["/tmp/luna-dev-002/target"]) {
    for (const profile of ["release", "debug"]) {
      const full = path.join(sharedTarget, profile, exeName);
      if (isExecutableFile(full)) {
        return full;
      }
    }
  }

  // Search system PATH.
  const pathEnv = process.env.PATH || "";
  for (const segment of pathEnv.split(path.delimiter)) {
    if (!segment) {
      continue;
    }
    const full = path.join(segment, exeName);
    if (isExecutableFile(full)) {
      return full;
    }
  }

  return null;
}

/**
 * Discover the Flurry `library/` sysroot directory.
 * @param {string} extensionPath
 * @param {string} configuredSysroot
 * @returns {string | null}
 */
function resolveSysroot(extensionPath, configuredSysroot) {
  if (configuredSysroot && configuredSysroot.trim().length > 0) {
    const trimmed = configuredSysroot.trim();
    return isValidSysroot(trimmed) ? trimmed : trimmed;
  }

  const roots = [];
  for (const folder of vscode.workspace.workspaceFolders || []) {
    roots.push(folder.uri.fsPath);
  }
  let dir = extensionPath;
  for (let i = 0; i < 4; i++) {
    roots.push(dir);
    const parent = path.dirname(dir);
    if (parent === dir) {
      break;
    }
    dir = parent;
  }

  for (const root of roots) {
    const candidate = path.join(root, "library");
    if (isValidSysroot(candidate)) {
      return candidate;
    }
  }
  return null;
}

function isValidSysroot(dirPath) {
  try {
    return (
      fs.statSync(path.join(dirPath, "builtin")).isDirectory() &&
      fs.statSync(path.join(dirPath, "meta")).isDirectory()
    );
  } catch (_) {
    return false;
  }
}

function isExecutableFile(filePath) {
  try {
    const st = fs.statSync(filePath);
    if (!st.isFile()) {
      return false;
    }
    if (process.platform !== "win32") {
      fs.accessSync(filePath, fs.constants.X_OK);
    }
    return true;
  } catch (_) {
    return false;
  }
}

function readFlurrySettings(resolvedSysroot) {
  const cfg = vscode.workspace.getConfiguration("flurry");
  const sysroot = cfg.get("anar.sysroot", "") || resolvedSysroot || "";
  return {
    anar: {
      enable: cfg.get("anar.enable", true),
      path: cfg.get("anar.path", ""),
      sysroot,
    },
    format: {
      enable: cfg.get("format.enable", true),
    },
  };
}

function buildClientCapabilities() {
  return {
    workspace: {
      workspaceFolders: true,
      configuration: true,
      didChangeConfiguration: { dynamicRegistration: false },
      didChangeWatchedFiles: { dynamicRegistration: false },
    },
    textDocument: {
      synchronization: {
        didSave: true,
      },
      hover: { contentFormat: ["markdown", "plaintext"] },
      completion: {
        completionItem: {
          snippetSupport: true,
          documentationFormat: ["markdown", "plaintext"],
        },
      },
      signatureHelp: {
        signatureInformation: {
          documentationFormat: ["markdown", "plaintext"],
        },
      },
      definition: { linkSupport: false },
      declaration: { linkSupport: false },
      references: {},
      documentHighlight: {},
      documentSymbol: { hierarchicalDocumentSymbolSupport: true },
      formatting: {},
      rename: { prepareSupport: true },
      foldingRange: {},
      publishDiagnostics: { relatedInformation: true },
      semanticTokens: {
        requests: { full: true },
        tokenTypes: DEFAULT_TOKEN_TYPES,
        tokenModifiers: DEFAULT_TOKEN_MODIFIERS,
        formats: ["relative"],
      },
    },
  };
}

function isFlurryDocument(doc) {
  return Boolean(doc && doc.languageId === LANGUAGE_ID && doc.uri.scheme === "file");
}

function toLspPosition(pos) {
  return { line: pos.line, character: pos.character };
}

function toLspRange(range) {
  return {
    start: toLspPosition(range.start),
    end: toLspPosition(range.end),
  };
}

function toTextDocumentPosition(doc, pos) {
  return {
    textDocument: { uri: doc.uri.toString() },
    position: toLspPosition(pos),
  };
}

function fromLspPosition(pos) {
  return new vscode.Position(pos.line || 0, pos.character || 0);
}

function fromLspRange(range) {
  return new vscode.Range(fromLspPosition(range.start), fromLspPosition(range.end));
}

function fromLspLocations(res) {
  if (!res) {
    return null;
  }
  const arr = Array.isArray(res) ? res : [res];
  return arr
    .filter((loc) => loc && (loc.uri || loc.targetUri))
    .map((loc) => {
      if (loc.targetUri) {
        return new vscode.Location(
          vscode.Uri.parse(loc.targetUri),
          fromLspRange(loc.targetSelectionRange || loc.targetRange)
        );
      }
      return new vscode.Location(vscode.Uri.parse(loc.uri), fromLspRange(loc.range));
    });
}

function fromLspCompletionItem(raw) {
  const item = new vscode.CompletionItem(
    raw.label,
    fromLspCompletionKind(raw.kind)
  );
  if (raw.detail) {
    item.detail = raw.detail;
  }
  if (raw.documentation) {
    item.documentation =
      typeof raw.documentation === "string"
        ? raw.documentation
        : new vscode.MarkdownString(raw.documentation.value || "");
  }
  if (raw.sortText) {
    item.sortText = raw.sortText;
  }
  if (raw.filterText) {
    item.filterText = raw.filterText;
  }
  if (raw.insertText) {
    item.insertText =
      raw.insertTextFormat === 2
        ? new vscode.SnippetString(raw.insertText)
        : raw.insertText;
  }
  return item;
}

function fromLspDocumentSymbol(raw) {
  const range = fromLspRange(raw.range);
  const selRange = raw.selectionRange ? fromLspRange(raw.selectionRange) : range;
  const sym = new vscode.DocumentSymbol(
    raw.name || "<unnamed>",
    raw.detail || "",
    fromLspSymbolKind(raw.kind),
    range,
    selRange
  );
  if (Array.isArray(raw.children)) {
    sym.children = raw.children.map(fromLspDocumentSymbol);
  }
  return sym;
}

function fromLspCompletionKind(lspKind) {
  // LSP CompletionItemKind is 1-based, while vscode.CompletionItemKind is 0-based (`lspKind - 1`).
  if (typeof lspKind === "number" && lspKind >= 1 && lspKind <= 25) {
    return lspKind - 1;
  }
  return vscode.CompletionItemKind.Text;
}

function fromLspSymbolKind(lspKind) {
  // LSP SymbolKind is 1-based (`File = 1`), while vscode.SymbolKind is 0-based (`File = 0`).
  if (typeof lspKind === "number" && lspKind >= 1 && lspKind <= 26) {
    return lspKind - 1;
  }
  return vscode.SymbolKind.Variable;
}

function updateStatusBar(text, tooltip) {
  if (!statusBarItem) {
    return;
  }
  statusBarItem.text = text;
  statusBarItem.tooltip = tooltip;
}

function logLine(line) {
  if (outputChannel) {
    outputChannel.appendLine(line);
  }
}

module.exports = {
  activate,
  deactivate,
  resolveLunaBinary,
  serverArguments,
  resolveSysroot,
};
