//! The context value type: a piece of material that may be given to a model,
//! together with what is known about where it came from.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::id::ContextId;
use crate::time::Timestamp;

/// Where a context was acquired from.
///
/// The `kind`/`reference` split is what storage persists, so adding a variant
/// never changes the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextSource {
    /// Piped into the CLI.
    Stdin,
    /// Read from a file on disk.
    File { path: PathBuf },
    /// Captured from the output of a command.
    Command { command: String },
    /// Produced by CtxC's own memory subsystem.
    Memory,
    /// Received over the HTTP API or an integration.
    Api { endpoint: String },
    /// Provenance not recorded.
    Unknown,
}

impl ContextSource {
    /// Stable discriminator, used for storage and for id derivation.
    pub fn kind(&self) -> &'static str {
        match self {
            ContextSource::Stdin => "stdin",
            ContextSource::File { .. } => "file",
            ContextSource::Command { .. } => "command",
            ContextSource::Memory => "memory",
            ContextSource::Api { .. } => "api",
            ContextSource::Unknown => "unknown",
        }
    }

    /// The variant's payload, if it has one.
    pub fn reference(&self) -> Option<String> {
        match self {
            ContextSource::File { path } => Some(path.to_string_lossy().into_owned()),
            ContextSource::Command { command } => Some(command.clone()),
            ContextSource::Api { endpoint } => Some(endpoint.clone()),
            ContextSource::Stdin | ContextSource::Memory | ContextSource::Unknown => None,
        }
    }

    /// Rebuild a source from its stored representation. Unknown kinds decay to
    /// [`ContextSource::Unknown`] instead of failing, so an older binary can
    /// still read a newer database.
    pub fn from_parts(kind: &str, reference: Option<String>) -> Self {
        match (kind, reference) {
            ("stdin", _) => ContextSource::Stdin,
            ("file", Some(path)) => ContextSource::File {
                path: PathBuf::from(path),
            },
            ("command", Some(command)) => ContextSource::Command { command },
            ("memory", _) => ContextSource::Memory,
            ("api", Some(endpoint)) => ContextSource::Api { endpoint },
            _ => ContextSource::Unknown,
        }
    }
}

/// What kind of material a context holds. Detection lands with the engine; this
/// is the vocabulary the router and the optimizers agree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    PlainText,
    Markdown,
    Json,
    Code,
    Log,
    Terminal,
    Binary,
    Unknown,
}

impl ContentType {
    /// Stable name used in storage and JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            ContentType::PlainText => "plain_text",
            ContentType::Markdown => "markdown",
            ContentType::Json => "json",
            ContentType::Code => "code",
            ContentType::Log => "log",
            ContentType::Terminal => "terminal",
            ContentType::Binary => "binary",
            ContentType::Unknown => "unknown",
        }
    }

    /// Inverse of [`ContentType::as_str`]; unrecognised names become
    /// [`ContentType::Unknown`] rather than an error.
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "plain_text" => ContentType::PlainText,
            "markdown" => ContentType::Markdown,
            "json" => ContentType::Json,
            "code" => ContentType::Code,
            "log" => ContentType::Log,
            "terminal" => ContentType::Terminal,
            "binary" => ContentType::Binary,
            _ => ContentType::Unknown,
        }
    }
}

/// Everything known about a context except its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMetadata {
    pub source: ContextSource,
    pub content_type: ContentType,
    /// Estimated token count, once something has estimated it.
    pub token_count: Option<u32>,
    /// Size of the content in bytes.
    pub byte_len: u64,
    pub created_at: Timestamp,
}

/// A unit of context: content plus provenance, addressed by a content-derived
/// id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub id: ContextId,
    pub content: String,
    pub metadata: ContextMetadata,
}

impl Context {
    /// Create a context, deriving its id from the source kind and content.
    pub fn new(
        source: ContextSource,
        content_type: ContentType,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        let id = ContextId::from_content(source.kind(), content.as_bytes());
        let metadata = ContextMetadata {
            source,
            content_type,
            token_count: None,
            byte_len: content.len() as u64,
            created_at: Timestamp::now(),
        };
        Context {
            id,
            content,
            metadata,
        }
    }

    /// Reassemble a context that was previously persisted.
    pub fn from_parts(id: ContextId, content: String, metadata: ContextMetadata) -> Self {
        Context {
            id,
            content,
            metadata,
        }
    }

    /// Attach an estimated token count.
    pub fn with_token_count(mut self, tokens: u32) -> Self {
        self.metadata.token_count = Some(tokens);
        self
    }

    /// The retrievable reference for this context.
    pub fn uri(&self) -> String {
        self.id.to_uri()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_derived_from_content() {
        let a = Context::new(ContextSource::Stdin, ContentType::PlainText, "hello");
        let b = Context::new(ContextSource::Stdin, ContentType::Json, "hello");
        assert_eq!(a.id, b.id, "content type must not affect identity");

        let c = Context::new(ContextSource::Memory, ContentType::PlainText, "hello");
        assert_ne!(a.id, c.id, "source kind must affect identity");
    }

    #[test]
    fn metadata_records_size() {
        let context = Context::new(ContextSource::Stdin, ContentType::PlainText, "h\u{e9}llo");
        assert_eq!(context.metadata.byte_len, 6, "bytes, not characters");
        assert_eq!(context.metadata.token_count, None);
        assert_eq!(
            context.clone().with_token_count(2).metadata.token_count,
            Some(2)
        );
    }

    #[test]
    fn source_roundtrips_through_storage_parts() {
        let sources = [
            ContextSource::Stdin,
            ContextSource::File {
                path: PathBuf::from("src/main.rs"),
            },
            ContextSource::Command {
                command: "git status".into(),
            },
            ContextSource::Memory,
            ContextSource::Api {
                endpoint: "/v1/context/optimize".into(),
            },
            ContextSource::Unknown,
        ];
        for source in sources {
            let restored = ContextSource::from_parts(source.kind(), source.reference());
            assert_eq!(restored, source);
        }
    }

    #[test]
    fn unknown_stored_kinds_decay_instead_of_failing() {
        assert_eq!(
            ContextSource::from_parts("from_the_future", Some("x".into())),
            ContextSource::Unknown
        );
        assert_eq!(
            ContentType::from_str_lossy("protobuf"),
            ContentType::Unknown
        );
    }

    #[test]
    fn content_type_names_roundtrip() {
        for content_type in [
            ContentType::PlainText,
            ContentType::Markdown,
            ContentType::Json,
            ContentType::Code,
            ContentType::Log,
            ContentType::Terminal,
            ContentType::Binary,
            ContentType::Unknown,
        ] {
            assert_eq!(
                ContentType::from_str_lossy(content_type.as_str()),
                content_type
            );
        }
    }
}
