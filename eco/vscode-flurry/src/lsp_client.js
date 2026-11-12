"use strict";

const { spawn } = require("child_process");
const { EventEmitter } = require("events");

const HEADER_DELIMITER = Buffer.from("\r\n\r\n", "ascii");

/**
 * Zero-dependency JSON-RPC 2.0 `Content-Length` stdio client for `anar`.
 */
class LspClient extends EventEmitter {
  /**
   * @param {object} options
   * @param {string} options.command
   * @param {string[]} [options.args]
   * @param {string} [options.cwd]
   * @param {NodeJS.ProcessEnv} [options.env]
   * @param {(line: string) => void} [options.logger]
   * @param {() => string} [options.getTraceLevel]
   */
  constructor(options) {
    super();
    this.command = options.command;
    this.args = Array.isArray(options.args) ? options.args : ["--stdio"];
    this.cwd = options.cwd || process.cwd();
    this.env = options.env || process.env;
    this.logger = typeof options.logger === "function" ? options.logger : () => {};
    this.getTraceLevel =
      typeof options.getTraceLevel === "function" ? options.getTraceLevel : () => "off";

    this.proc = null;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.stopping = false;
    this.serverCapabilities = null;
    this.serverInfo = null;
  }

  isRunning() {
    return Boolean(this.proc && !this.proc.killed && this.proc.exitCode === null);
  }

  start() {
    if (this.proc) {
      return;
    }
    this.stopping = false;
    this.buffer = Buffer.alloc(0);

    const child = spawn(this.command, this.args, {
      cwd: this.cwd,
      env: this.env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.proc = child;

    child.stdout.on("data", (chunk) => {
      this._onStdoutData(chunk);
    });

    child.stderr.on("data", (chunk) => {
      const text = chunk.toString("utf8").trimEnd();
      if (text) {
        this.logger(`[anar stderr] ${text}`);
      }
    });

    child.on("error", (err) => {
      this.logger(`[anar process error] ${err.message}`);
      this._rejectAllPending(err);
      this.emit("error", err);
    });

    child.on("exit", (code, signal) => {
      const wasStopping = this.stopping;
      this.proc = null;
      this._rejectAllPending(
        new Error(`anar exited (code=${code}, signal=${signal || "none"})`)
      );
      this.emit("exit", { code, signal, intentional: wasStopping });
    });
  }

  /**
   * Perform the LSP `initialize` + `initialized` handshake.
   * @param {object} params
   * @returns {Promise<object>}
   */
  async initialize(params) {
    this.start();
    const result = await this.sendRequest("initialize", params);
    this.serverCapabilities = (result && result.capabilities) || {};
    this.serverInfo = (result && result.serverInfo) || null;
    this.sendNotification("initialized", {});
    return result;
  }

  /**
   * Send a JSON-RPC request, optionally bound to a VS Code CancellationToken.
   * @param {string} method
   * @param {any} params
   * @param {{ isCancellationRequested?: boolean, onCancellationRequested?: (cb: () => void) => { dispose?: () => void } }} [token]
   * @returns {Promise<any>}
   */
  sendRequest(method, params, token) {
    if (!this.isRunning()) {
      return Promise.reject(new Error("anar language server is not running"));
    }
    if (token && token.isCancellationRequested) {
      return Promise.reject(createCancelledError());
    }

    const id = this.nextId++;
    const payload = {
      jsonrpc: "2.0",
      id,
      method,
      params: params === undefined ? null : params,
    };

    return new Promise((resolve, reject) => {
      let cancelDisposable = null;
      if (token && typeof token.onCancellationRequested === "function") {
        cancelDisposable = token.onCancellationRequested(() => {
          if (this.pending.has(id)) {
            this.sendNotification("$/cancelRequest", { id });
          }
        });
      }

      this.pending.set(id, {
        method,
        resolve: (value) => {
          if (cancelDisposable && typeof cancelDisposable.dispose === "function") {
            cancelDisposable.dispose();
          }
          resolve(value);
        },
        reject: (err) => {
          if (cancelDisposable && typeof cancelDisposable.dispose === "function") {
            cancelDisposable.dispose();
          }
          reject(err);
        },
      });

      this._traceOutgoing("request", method, id, params);
      this._writeMessage(payload);
    });
  }

  /**
   * Send a JSON-RPC notification.
   * @param {string} method
   * @param {any} params
   */
  sendNotification(method, params) {
    if (!this.isRunning()) {
      return;
    }
    const payload = {
      jsonrpc: "2.0",
      method,
      params: params === undefined ? {} : params,
    };
    this._traceOutgoing("notification", method, null, params);
    this._writeMessage(payload);
  }

  /**
   * Gracefully shut down `anar` via `shutdown` + `exit`, falling back to SIGTERM after timeout.
   * @param {number} [timeoutMs]
   * @returns {Promise<void>}
   */
  async stop(timeoutMs = 2000) {
    if (!this.proc) {
      return;
    }
    this.stopping = true;
    const child = this.proc;

    const exitPromise = new Promise((resolve) => {
      if (child.exitCode !== null || child.killed) {
        resolve();
        return;
      }
      child.once("exit", () => resolve());
    });

    try {
      await Promise.race([
        this.sendRequest("shutdown", null),
        new Promise((_, reject) =>
          setTimeout(() => reject(new Error("shutdown timed out")), timeoutMs)
        ),
      ]);
      this.sendNotification("exit", null);
    } catch (_) {
      try {
        child.kill();
      } catch (_) {
        // Ignore kill error on already exited child.
      }
    }

    await Promise.race([
      exitPromise,
      new Promise((resolve) => {
        setTimeout(() => {
          if (child.exitCode === null && !child.killed) {
            try {
              child.kill("SIGKILL");
            } catch (_) {
              // Ignore
            }
          }
          resolve();
        }, timeoutMs);
      }),
    ]);
    this.proc = null;
  }

  _writeMessage(obj) {
    if (!this.proc || !this.proc.stdin || this.proc.stdin.destroyed) {
      return;
    }
    const body = Buffer.from(JSON.stringify(obj), "utf8");
    const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii");
    this.proc.stdin.write(Buffer.concat([header, body]));
  }

  _onStdoutData(chunk) {
    this.buffer =
      this.buffer.length === 0 ? Buffer.from(chunk) : Buffer.concat([this.buffer, chunk]);

    while (true) {
      const headerEnd = this.buffer.indexOf(HEADER_DELIMITER);
      if (headerEnd === -1) {
        return;
      }

      const headerStr = this.buffer.subarray(0, headerEnd).toString("ascii");
      const contentLength = parseContentLength(headerStr);
      if (contentLength === null || contentLength < 0) {
        this.logger(`[anar protocol error] invalid LSP header: ${JSON.stringify(headerStr)}`);
        this.buffer = this.buffer.subarray(headerEnd + HEADER_DELIMITER.length);
        continue;
      }

      const messageStart = headerEnd + HEADER_DELIMITER.length;
      const messageEnd = messageStart + contentLength;
      if (this.buffer.length < messageEnd) {
        return;
      }

      const rawBody = this.buffer.subarray(messageStart, messageEnd).toString("utf8");
      this.buffer =
        messageEnd === this.buffer.length
          ? Buffer.alloc(0)
          : Buffer.from(this.buffer.subarray(messageEnd));

      let msg;
      try {
        msg = JSON.parse(rawBody);
      } catch (err) {
        this.logger(`[anar protocol error] invalid JSON payload: ${err.message}`);
        continue;
      }
      this._dispatchMessage(msg);
    }
  }

  _dispatchMessage(msg) {
    if (!msg || typeof msg !== "object") {
      return;
    }

    // Response to an outgoing request
    if (Object.prototype.hasOwnProperty.call(msg, "id") && !msg.method) {
      const entry = this.pending.get(msg.id);
      if (!entry) {
        return;
      }
      this.pending.delete(msg.id);
      this._traceIncoming("response", entry.method, msg.id, msg.error || msg.result);
      if (msg.error) {
        const err = new Error(msg.error.message || "LSP request failed");
        err.code = msg.error.code;
        err.data = msg.error.data;
        entry.reject(err);
      } else {
        entry.resolve(msg.result);
      }
      return;
    }

    // Server-initiated request
    if (Object.prototype.hasOwnProperty.call(msg, "id") && typeof msg.method === "string") {
      this._traceIncoming("server-request", msg.method, msg.id, msg.params);
      this._writeMessage({
        jsonrpc: "2.0",
        id: msg.id,
        result: null,
      });
      return;
    }

    // Server notification
    if (typeof msg.method === "string") {
      this._traceIncoming("notification", msg.method, null, msg.params);
      this.emit("notification", msg.method, msg.params);
      this.emit(`notification:${msg.method}`, msg.params);
    }
  }

  _rejectAllPending(err) {
    for (const [, entry] of this.pending) {
      entry.reject(err);
    }
    this.pending.clear();
  }

  _traceOutgoing(kind, method, id, params) {
    const level = this.getTraceLevel();
    if (level === "off") {
      return;
    }
    const suffix = id !== null ? ` #${id}` : "";
    if (level === "verbose") {
      this.logger(`-> [${kind}] ${method}${suffix} ${ safeJson(params) }`);
    } else {
      this.logger(`-> [${kind}] ${method}${suffix}`);
    }
  }

  _traceIncoming(kind, method, id, payload) {
    const level = this.getTraceLevel();
    if (level === "off") {
      return;
    }
    const suffix = id !== null ? ` #${id}` : "";
    if (level === "verbose") {
      this.logger(`<- [${kind}] ${method}${suffix} ${ safeJson(payload) }`);
    } else {
      this.logger(`<- [${kind}] ${method}${suffix}`);
    }
  }
}

function parseContentLength(headerText) {
  const lines = headerText.split("\r\n");
  for (const line of lines) {
    const colon = line.indexOf(":");
    if (colon === -1) {
      continue;
    }
    const key = line.slice(0, colon).trim().toLowerCase();
    if (key === "content-length") {
      const value = Number.parseInt(line.slice(colon + 1).trim(), 10);
      return Number.isFinite(value) ? value : null;
    }
  }
  return null;
}

function createCancelledError() {
  const err = new Error("Request cancelled");
  err.code = -32800;
  return err;
}

function safeJson(val) {
  try {
    return JSON.stringify(val);
  } catch (_) {
    return "[unserializable]";
  }
}

module.exports = {
  LspClient,
  parseContentLength,
};
