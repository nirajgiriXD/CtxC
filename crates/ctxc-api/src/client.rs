//! A small blocking client for the local API.
//!
//! The CLI needs to ask a daemon on loopback a handful of questions. That does
//! not justify an async HTTP stack with TLS, connection pooling and its
//! dependency tree, so this speaks the part of HTTP/1.1 that a localhost
//! request actually uses: one request, one response, `Connection: close`.
//!
//! It is deliberately not a general-purpose client — no redirects, no chunked
//! encoding, no keep-alive — and it only ever talks to 127.0.0.1.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::state::AccessToken;

/// How long to wait for a daemon that is not answering.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A failure talking to the daemon.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the daemon is not answering on port {port}")]
    Unreachable {
        port: u16,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to talk to the daemon")]
    Transport(#[from] std::io::Error),

    #[error("the daemon sent a response this build cannot read")]
    Malformed,

    #[error("{message}")]
    Api {
        status: u16,
        message: String,
        hint: Option<String>,
    },
}

impl ClientError {
    pub fn hint(&self) -> Option<String> {
        match self {
            ClientError::Unreachable { .. } => {
                Some("start it with `ctxc start`, or check `ctxc status --daemon`".into())
            }
            ClientError::Api { hint, .. } => hint.clone(),
            ClientError::Transport(_) | ClientError::Malformed => None,
        }
    }

    /// Whether this means "no daemon is running" rather than "it said no".
    pub fn is_unreachable(&self) -> bool {
        matches!(self, ClientError::Unreachable { .. })
    }
}

pub type Result<T, E = ClientError> = std::result::Result<T, E>;

/// A client for one daemon.
pub struct Client {
    port: u16,
    token: Option<AccessToken>,
}

impl Client {
    pub fn new(port: u16, token: Option<AccessToken>) -> Self {
        Client { port, token }
    }

    /// Whether a daemon is answering on this port.
    ///
    /// Used as the liveness check for the lockfile: a recorded process id can
    /// be reused by an unrelated program, but a CtxC daemon answering `/v1/health`
    /// on the recorded port is proof.
    pub fn is_alive(&self) -> bool {
        self.get::<crate::routes::Health>("/v1/health")
            .map(|health| health.status == "ok")
            .unwrap_or(false)
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send::<(), T>("GET", path, None)
    }

    pub fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        self.send("POST", path, Some(body))
    }

    pub fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send::<(), T>("POST", path, None)
    }

    pub fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send::<(), T>("DELETE", path, None)
    }

    fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port);
        let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(|source| {
            ClientError::Unreachable {
                port: self.port,
                source,
            }
        })?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;

        let payload = match body {
            Some(body) => serde_json::to_vec(body).map_err(|_| ClientError::Malformed)?,
            None => Vec::new(),
        };

        let mut request = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Connection: close\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n",
            self.port,
            payload.len()
        );
        if let Some(token) = &self.token {
            request.push_str(&format!("Authorization: Bearer {}\r\n", token.as_str()));
        }
        request.push_str("\r\n");

        stream.write_all(request.as_bytes())?;
        stream.write_all(&payload)?;
        stream.flush()?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        parse(&response)
    }
}

/// Split a response into its status and body, then decode the body.
fn parse<T: DeserializeOwned>(response: &[u8]) -> Result<T> {
    let split = find(response, b"\r\n\r\n").ok_or(ClientError::Malformed)?;
    let head = std::str::from_utf8(&response[..split]).map_err(|_| ClientError::Malformed)?;
    let body = &response[split + 4..];

    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or(ClientError::Malformed)?;

    if (200..300).contains(&status) {
        return serde_json::from_slice(body).map_err(|_| ClientError::Malformed);
    }

    // Failures carry the same shape as everything else, so the CLI can show the
    // daemon's own message and hint rather than inventing one.
    let error: crate::routes::ApiErrorBody =
        serde_json::from_slice(body).unwrap_or(crate::routes::ApiErrorBody {
            error: format!("the daemon returned {status}"),
            hint: None,
        });

    Err(ClientError::Api {
        status,
        message: error.error,
        hint: error.hint,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Body {
        value: u32,
    }

    fn response(status: u16, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status} SOMETHING\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn a_successful_response_is_decoded() {
        let parsed: Body = parse(&response(200, r#"{"value":7}"#)).unwrap();
        assert_eq!(parsed, Body { value: 7 });
    }

    #[test]
    fn an_error_response_carries_the_daemons_message() {
        let raw = response(404, r#"{"error":"no project matches x","hint":"try list"}"#);
        let error = parse::<Body>(&raw).unwrap_err();

        match error {
            ClientError::Api {
                status,
                message,
                hint,
            } => {
                assert_eq!(status, 404);
                assert_eq!(message, "no project matches x");
                assert_eq!(hint.as_deref(), Some("try list"));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn an_error_without_a_body_still_reports_its_status() {
        let error = parse::<Body>(&response(500, "not json at all")).unwrap_err();
        assert!(error.to_string().contains("500"));
    }

    #[test]
    fn a_truncated_response_is_rejected() {
        assert!(matches!(
            parse::<Body>(b"HTTP/1.1 200 OK\r\nContent-Type: application/json"),
            Err(ClientError::Malformed)
        ));
    }

    #[test]
    fn a_closed_port_reports_that_the_daemon_is_unreachable() {
        // Port 1 on loopback is not something a daemon binds.
        let client = Client::new(1, None);
        assert!(!client.is_alive());

        let error = client.get::<Body>("/v1/health").unwrap_err();
        assert!(error.is_unreachable());
        assert!(error.hint().unwrap().contains("ctxc start"));
    }
}
