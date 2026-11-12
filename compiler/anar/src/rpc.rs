//! JSON-RPC 2.0 `Content-Length` stdio framing for the Language Server Protocol.

use std::io::{self, BufRead, Write};

use crate::json::JsonValue;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const SERVER_NOT_INITIALIZED: i64 = -32002;
pub const REQUEST_CANCELLED: i64 = -32800;

pub const ERR_INVALID_REQUEST: i64 = INVALID_REQUEST;
pub const ERR_METHOD_NOT_FOUND: i64 = METHOD_NOT_FOUND;
pub const ERR_INVALID_PARAMS: i64 = INVALID_PARAMS;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RequestId {
    Int(i64),
    Str(String),
    Null,
}

impl RequestId {
    pub fn from_json(val: &JsonValue) -> Option<Self> {
        match val {
            JsonValue::Int(n) => Some(RequestId::Int(*n)),
            JsonValue::Str(s) => Some(RequestId::Str(s.clone())),
            JsonValue::Null => Some(RequestId::Null),
            _ => None,
        }
    }

    pub fn to_json(&self) -> JsonValue {
        match self {
            RequestId::Int(n) => JsonValue::Int(*n),
            RequestId::Str(s) => JsonValue::Str(s.clone()),
            RequestId::Null => JsonValue::Null,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RpcMessage {
    Request {
        id: RequestId,
        method: String,
        params: JsonValue,
    },
    Notification {
        method: String,
        params: JsonValue,
    },
    Response {
        id: RequestId,
        result: Option<JsonValue>,
        error: Option<RpcError>,
    },
}

#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<JsonValue>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut fields = vec![
            ("code".to_owned(), JsonValue::Int(self.code)),
            ("message".to_owned(), JsonValue::Str(self.message.clone())),
        ];
        if let Some(data) = &self.data {
            fields.push(("data".to_owned(), data.clone()));
        }
        JsonValue::Object(fields)
    }
}

impl RpcMessage {
    pub fn from_json(value: JsonValue) -> Result<Self, RpcError> {
        let _ = value.as_object().ok_or_else(|| {
            RpcError::new(INVALID_REQUEST, "JSON-RPC payload must be a JSON object")
        })?;
        let id = value.get("id").and_then(RequestId::from_json);
        if let Some(method) = value.get_str("method") {
            let method = method.to_owned();
            let params = value.get("params").cloned().unwrap_or(JsonValue::Null);
            if let Some(id) = id {
                Ok(RpcMessage::Request { id, method, params })
            } else {
                Ok(RpcMessage::Notification { method, params })
            }
        } else if let Some(id) = id {
            let result = value.get("result").cloned();
            let error = value.get("error").map(|err| RpcError {
                code: err.get_i64("code").unwrap_or(INTERNAL_ERROR),
                message: err
                    .get_str("message")
                    .unwrap_or("unknown RPC error")
                    .to_owned(),
                data: err.get("data").cloned(),
            });
            Ok(RpcMessage::Response { id, result, error })
        } else {
            Err(RpcError::new(
                INVALID_REQUEST,
                "JSON-RPC object missing 'method' or 'id'",
            ))
        }
    }

    pub fn ok_response(id: RequestId, result: JsonValue) -> JsonValue {
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", id.to_json()),
            ("result", result),
        ])
    }

    pub fn error_response(id: RequestId, error: RpcError) -> JsonValue {
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("id", id.to_json()),
            ("error", error.to_json()),
        ])
    }

    pub fn notification(method: &str, params: JsonValue) -> JsonValue {
        JsonValue::object([
            ("jsonrpc", JsonValue::from("2.0")),
            ("method", JsonValue::from(method)),
            ("params", params),
        ])
    }
}

/// Read one `Content-Length`-framed LSP message from `reader`.
/// Returns `Ok(None)` cleanly on EOF before any header bytes.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();
    let mut saw_any_header = false;

    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            if !saw_any_header {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "EOF while reading LSP headers",
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if saw_any_header {
                break;
            }
            continue;
        }
        saw_any_header = true;
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length header")
                })?);
            }
        }
    }

    let len = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let body = String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(body))
}

/// Write a `Content-Length`-framed JSON-RPC payload to `writer` and flush.
pub fn write_message<W: Write>(writer: &mut W, payload: &JsonValue) -> io::Result<()> {
    let body = payload.to_json_string();
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()
}

pub fn write_rpc_payload<W: Write>(writer: &mut W, payload: &JsonValue) -> io::Result<()> {
    write_message(writer, payload)
}

pub fn read_rpc_message<R: BufRead>(reader: &mut R) -> io::Result<Option<RpcMessage>> {
    let Some(raw) = read_message(reader)? else {
        return Ok(None);
    };
    let val = JsonValue::parse(&raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let msg = RpcMessage::from_json(val)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.message))?;
    Ok(Some(msg))
}

pub fn send_response<W: Write>(writer: &mut W, id: RequestId, result: JsonValue) -> io::Result<()> {
    write_message(writer, &RpcMessage::ok_response(id, result))
}

pub fn send_error<W: Write>(
    writer: &mut W,
    id: RequestId,
    code: i64,
    message: &str,
) -> io::Result<()> {
    write_message(
        writer,
        &RpcMessage::error_response(id, RpcError::new(code, message)),
    )
}

pub fn send_notification<W: Write>(
    writer: &mut W,
    method: &str,
    params: JsonValue,
) -> io::Result<()> {
    write_message(writer, &RpcMessage::notification(method, params))
}
