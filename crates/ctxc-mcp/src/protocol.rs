//! JSON-RPC 2.0, and the slice of MCP a tools server needs.
//!
//! MCP is JSON-RPC over a transport, and a server that only offers tools uses a
//! handful of methods: `initialize`, `tools/list`, `tools/call`, `ping`. That is
//! small enough to implement directly, which keeps CtxC free of a protocol
//! library whose release cadence it does not control — the same reasoning that
//! put a hand-written HTTP client in `ctxc-api`.
//!
//! ```text
//! stdin  --> Request  --> dispatch --> Response --> stdout
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Protocol revisions this server can speak.
///
/// Newest first: an `initialize` that asks for something unknown gets the first
/// of these, which is the specified way to say "I speak this instead".
pub const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The version offered when a client asks for one this build does not know.
pub fn preferred_version() -> &'static str {
    SUPPORTED_VERSIONS[0]
}

/// Agree on a protocol version.
///
/// Returning the client's version when it is one we speak keeps older clients
/// working; anything else gets ours, and the client decides whether to proceed.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|wanted| {
            SUPPORTED_VERSIONS
                .iter()
                .find(|version| **version == wanted)
                .copied()
        })
        .unwrap_or_else(preferred_version)
}

/// One incoming message.
///
/// A message with no `id` is a notification: it is acted on, and never
/// answered. Replying to one is a protocol violation that some clients treat as
/// fatal.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl Request {
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// A named parameter, if it was given.
    pub fn param(&self, name: &str) -> Option<&Value> {
        self.params.as_ref()?.get(name)
    }
}

/// Standard JSON-RPC error codes, plus what each means here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
}

impl ErrorCode {
    pub fn code(self) -> i32 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
        }
    }
}

/// One outgoing message.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
}

impl Response {
    pub fn success(id: Value, result: Value) -> Self {
        Response {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: Value, code: ErrorCode, message: impl Into<String>) -> Self {
        Response {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(ResponseError {
                code: code.code(),
                message: message.into(),
            }),
        }
    }
}

/// A tool, as advertised to a client.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// What a tool call produced.
///
/// A tool that failed reports `is_error` rather than a JSON-RPC error: the
/// model is supposed to see what went wrong and try something else, and a
/// transport-level error is hidden from it.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(text: impl Into<String>) -> Self {
        ToolResult {
            text: text.into(),
            is_error: false,
        }
    }

    pub fn failed(text: impl Into<String>) -> Self {
        ToolResult {
            text: text.into(),
            is_error: true,
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "content": [{ "type": "text", "text": self.text }],
            "isError": self.is_error,
        })
    }
}

/// Build a JSON Schema object for a tool's parameters.
///
/// Written by hand rather than derived: the descriptions here are read by a
/// model deciding whether to call the tool, so they are prose worth writing
/// deliberately, not a byproduct of a struct definition.
pub fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_without_an_id_is_a_notification() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .unwrap();

        assert!(request.is_notification());
        assert_eq!(request.method, "notifications/initialized");
    }

    #[test]
    fn an_id_of_zero_is_still_an_id() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#).unwrap();

        assert!(
            !request.is_notification(),
            "id 0 is falsy in JavaScript and a perfectly good id here"
        );
    }

    #[test]
    fn parameters_are_read_by_name() {
        let request: Request = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ctxc_search"}}"#,
        )
        .unwrap();

        assert_eq!(request.param("name").unwrap(), "ctxc_search");
        assert!(request.param("missing").is_none());
    }

    #[test]
    fn a_version_we_speak_is_echoed_back() {
        for version in SUPPORTED_VERSIONS {
            assert_eq!(negotiate(Some(version)), *version);
        }
    }

    #[test]
    fn an_unknown_version_gets_our_newest() {
        assert_eq!(negotiate(Some("1999-01-01")), preferred_version());
        assert_eq!(negotiate(None), preferred_version());
    }

    #[test]
    fn a_response_carries_a_result_or_an_error_but_never_both() {
        let ok = serde_json::to_value(Response::success(json!(1), json!({"tools": []}))).unwrap();
        assert!(ok.get("result").is_some());
        assert!(ok.get("error").is_none(), "{ok}");

        let bad = serde_json::to_value(Response::failure(
            json!(1),
            ErrorCode::MethodNotFound,
            "no such method",
        ))
        .unwrap();
        assert_eq!(bad["error"]["code"], -32601);
        assert!(bad.get("result").is_none(), "{bad}");
    }

    #[test]
    fn a_failed_tool_is_a_result_not_a_protocol_error() {
        let value = ToolResult::failed("that project is not indexed").to_value();

        assert_eq!(value["isError"], true);
        assert_eq!(value["content"][0]["type"], "text");
        assert!(
            value["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("not indexed"),
            "the model has to be able to read what went wrong"
        );
    }

    #[test]
    fn schemas_refuse_parameters_they_did_not_declare() {
        let built = schema(json!({ "query": { "type": "string" } }), &["query"]);

        assert_eq!(built["type"], "object");
        assert_eq!(built["required"][0], "query");
        assert_eq!(
            built["additionalProperties"], false,
            "a typo in a parameter name should be caught, not ignored"
        );
    }
}
