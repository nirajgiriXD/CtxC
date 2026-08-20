//! CtxC as an MCP server.
//!
//! Agents that speak the Model Context Protocol get CtxC's search, retrieval,
//! optimization and memory as tools, over stdio, with no HTTP and no daemon
//! required.
//!
//! ```text
//! agent --stdin--> Server --> Tools --> engine / index / store
//!       <--stdout--        (the same code the CLI calls)
//! ```
//!
//! This is an integration layer. Nothing in the engine knows it exists, and
//! removing this crate removes nothing but the adapter — which is the same rule
//! the HTTP API and the CLI follow, and the reason all three can never drift
//! into having different behaviour.
//!
//! **stdout belongs to the protocol.** A stray `println!` anywhere in the
//! process corrupts the stream and the client disconnects, so everything this
//! server has to say goes to stderr.

pub mod protocol;
pub mod tools;

pub use protocol::{Request, Response, ToolDefinition, ToolResult};
pub use tools::Tools;

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use protocol::{negotiate, ErrorCode};

/// The name a client shows for this server.
pub const SERVER_NAME: &str = "ctxc";

/// Handles MCP messages.
///
/// Stateless between messages. The handshake is not enforced — a client that
/// calls a tool before `initialize` gets the answer rather than a lecture,
/// which costs nothing and is one fewer way for a session to fail.
pub struct Server {
    tools: Tools,
}

impl Server {
    pub fn new(tools: Tools) -> Self {
        Server { tools }
    }

    /// Read messages from `input` and write answers to `output` until the
    /// stream ends.
    ///
    /// End of input is how an MCP client says goodbye — it closes the pipe — so
    /// that is a clean exit, not a failure.
    pub fn serve(&self, input: impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let Some(response) = self.handle_line(&line) else {
                continue;
            };

            // One JSON object per line, flushed immediately: the client is
            // waiting on this and will not see a buffered write.
            serde_json::to_writer(&mut *output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }

        Ok(())
    }

    /// Handle one line. `None` means nothing should be written back.
    pub fn handle_line(&self, line: &str) -> Option<Response> {
        let request: Request = match serde_json::from_str(line) {
            Ok(request) => request,
            // A parse failure has no id to answer, so the specified reply uses
            // a null one.
            Err(err) => {
                tracing::warn!(error = %err, "could not parse a message");
                return Some(Response::failure(
                    Value::Null,
                    ErrorCode::ParseError,
                    format!("could not parse that message: {err}"),
                ));
            }
        };

        let is_notification = request.is_notification();
        let id = request.id.clone().unwrap_or(Value::Null);
        let response = self.handle(request);

        // Notifications are acted on and never answered. Replying to one is a
        // protocol violation that some clients treat as fatal.
        if is_notification {
            return None;
        }
        Some(match response {
            Ok(result) => Response::success(id, result),
            Err((code, message)) => Response::failure(id, code, message),
        })
    }

    fn handle(&self, request: Request) -> Result<Value, (ErrorCode, String)> {
        match request.method.as_str() {
            "initialize" => Ok(self.initialize(&request)),

            // Sent by the client once it is ready. Nothing to do but note it.
            "notifications/initialized" => {
                tracing::debug!("client finished initializing");
                Ok(Value::Null)
            }

            "ping" => Ok(json!({})),

            "tools/list" => Ok(json!({ "tools": tools::definitions() })),

            "tools/call" => self.call_tool(&request),

            // Cancellation and progress are optional, and a server that
            // ignores them is still correct. Saying so beats an error.
            other if other.starts_with("notifications/") => {
                tracing::debug!(method = other, "ignoring a notification");
                Ok(Value::Null)
            }

            other => Err((
                ErrorCode::MethodNotFound,
                format!("this server does not implement `{other}`"),
            )),
        }
    }

    fn initialize(&self, request: &Request) -> Value {
        let requested = request.param("protocolVersion").and_then(Value::as_str);
        let version = negotiate(requested);

        let client = request
            .param("clientInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("an unnamed client");
        tracing::info!(client, version, "MCP session started");

        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": ctxc_core::VERSION,
            },
            "instructions": "\
        CtxC indexes this project and compiles context for it. Use ctxc_search to find \
        relevant code, ctxc_optimize to shrink noisy command output before reading it, \
        and ctxc_retrieve to recover anything optimization removed. Token counts are \
        estimates.",
        })
    }

    fn call_tool(&self, request: &Request) -> Result<Value, (ErrorCode, String)> {
        let name = request.param("name").and_then(Value::as_str).ok_or((
            ErrorCode::InvalidParams,
            "a tool call needs a `name`".to_string(),
        ))?;

        // Absent arguments are an empty object, not an error: a tool whose
        // parameters are all optional is legitimately called with none.
        let arguments = request
            .param("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        tracing::debug!(tool = name, "tool call");
        Ok(self.tools.call(name, &arguments).to_value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn server() -> Server {
        Server::new(Tools::new(
            ctxc_core::Config::default(),
            PathBuf::from(":memory:"),
            PathBuf::from("."),
        ))
    }

    fn ask(server: &Server, line: &str) -> Value {
        let response = server.handle_line(line).expect("a request is answered");
        serde_json::to_value(response).unwrap()
    }

    #[test]
    fn initialize_agrees_on_a_version_and_names_the_server() {
        let server = server();
        let response = ask(
            &server,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","clientInfo":{"name":"test"}}}"#,
        );

        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], "ctxc");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn an_unknown_protocol_version_gets_ours_rather_than_an_error() {
        let server = server();
        let response = ask(
            &server,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );

        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(
            response["result"]["protocolVersion"],
            protocol::preferred_version()
        );
    }

    #[test]
    fn a_notification_is_never_answered() {
        let server = server();

        assert!(
            server
                .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none(),
            "answering a notification is a protocol violation"
        );
        assert!(server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#)
            .is_none());
    }

    #[test]
    fn tools_are_listed_with_their_schemas() {
        let server = server();
        let response = ask(&server, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);

        let listed = response["result"]["tools"].as_array().unwrap();
        assert_eq!(listed.len(), tools::definitions().len());
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "ctxc_search" && tool["inputSchema"].is_object()));
    }

    #[test]
    fn ping_is_answered() {
        let server = server();
        let response = ask(&server, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#);

        assert!(response["result"].is_object());
        assert!(response.get("error").is_none());
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let server = server();
        let response = ask(
            &server,
            r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#,
        );

        assert_eq!(response["error"]["code"], -32601);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("resources/list"));
    }

    #[test]
    fn a_malformed_message_is_answered_with_a_null_id() {
        let server = server();
        let response = ask(&server, "{not json");

        assert_eq!(response["error"]["code"], -32700);
        assert_eq!(
            response["id"],
            Value::Null,
            "there is no id to answer, and the specified reply uses null"
        );
    }

    #[test]
    fn a_tool_call_without_a_name_is_an_invalid_params_error() {
        let server = server();
        let response = ask(
            &server,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{}}"#,
        );

        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn a_tool_that_fails_reports_it_in_the_result_so_the_model_can_see_it() {
        let server = server();
        let response = ask(
            &server,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"ctxc_nope"}}"#,
        );

        assert!(
            response.get("error").is_none(),
            "a failed tool is not a transport failure: {response}"
        );
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn a_conversation_over_a_pipe_produces_one_json_object_per_line() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
        );

        let mut output = Vec::new();
        server()
            .serve(std::io::BufReader::new(input.as_bytes()), &mut output)
            .unwrap();

        let text = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(
            lines.len(),
            2,
            "the notification and the blank line are not answered: {text}"
        );
        for line in lines {
            let parsed: Value = serde_json::from_str(line).expect("each line is one JSON object");
            assert_eq!(parsed["jsonrpc"], "2.0");
        }
    }

    #[test]
    fn closing_the_pipe_is_a_clean_exit() {
        let mut output = Vec::new();
        let result = server().serve(std::io::BufReader::new(&b""[..]), &mut output);

        assert!(result.is_ok(), "a client hanging up is not a failure");
        assert!(output.is_empty());
    }
}
