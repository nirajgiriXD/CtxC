//! Context identifiers.
//!
//! Ids are content addressed: the same bytes always produce the same id, so
//! storing a context twice is naturally idempotent and de-duplicating. Ids are
//! the first 128 bits of a BLAKE3 hash, rendered as 32 lowercase hex chars.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Hash bytes, rendered as 64 hex characters.
///
/// Used as a file content fingerprint by the indexer: the full digest rather
/// than a context id, because a collision here would mean skipping a file that
/// actually changed.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Scheme used when a context is referenced from inside optimized output.
pub const URI_PREFIX: &str = "ctxc://context/";

/// Number of hex characters in a rendered [`ContextId`].
pub const ID_LEN: usize = 32;

/// A stable, content-derived identifier for a context.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContextId(String);

impl ContextId {
    /// Derive an id from a domain tag and the context bytes.
    ///
    /// The tag keeps identical bytes acquired from different kinds of source
    /// from colliding (a file named `x` and a command printing `x`).
    pub fn from_content(tag: &str, content: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(tag.as_bytes());
        hasher.update(&[0]);
        hasher.update(content);
        let hash = hasher.finalize();

        let mut hex = String::with_capacity(ID_LEN);
        for byte in &hash.as_bytes()[..ID_LEN / 2] {
            hex.push_str(&format!("{byte:02x}"));
        }
        ContextId(hex)
    }

    /// Parse an id, accepting either the bare id or a `ctxc://context/<id>` URI.
    pub fn parse(input: &str) -> Result<Self> {
        let candidate = input.strip_prefix(URI_PREFIX).unwrap_or(input).trim();
        let valid = candidate.len() == ID_LEN
            && candidate
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valid {
            Ok(ContextId(candidate.to_owned()))
        } else {
            Err(Error::InvalidContextId(input.to_owned()))
        }
    }

    /// The id as it is stored and displayed.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The retrievable reference embedded in optimized context.
    pub fn to_uri(&self) -> String {
        format!("{URI_PREFIX}{}", self.0)
    }
}

impl fmt::Display for ContextId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ContextId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        ContextId::parse(s)
    }
}

impl TryFrom<String> for ContextId {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        ContextId::parse(&value)
    }
}

impl From<ContextId> for String {
    fn from(value: ContextId) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_same_id() {
        let a = ContextId::from_content("file", b"hello");
        let b = ContextId::from_content("file", b"hello");
        assert_eq!(a, b);
        assert_eq!(a.as_str().len(), ID_LEN);
    }

    #[test]
    fn tag_separates_identical_bytes() {
        let file = ContextId::from_content("file", b"hello");
        let command = ContextId::from_content("command", b"hello");
        assert_ne!(file, command);
    }

    #[test]
    fn tag_boundary_is_unambiguous() {
        // Without the separator byte these two would hash the same stream.
        let a = ContextId::from_content("ab", b"c");
        let b = ContextId::from_content("a", b"bc");
        assert_ne!(a, b);
    }

    #[test]
    fn parses_bare_and_uri_forms() {
        let id = ContextId::from_content("file", b"hello");
        assert_eq!(ContextId::parse(id.as_str()).unwrap(), id);
        assert_eq!(ContextId::parse(&id.to_uri()).unwrap(), id);
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in [
            "",
            "xyz",
            "ABCDEF01234567890123456789ABCDEF",
            &"0".repeat(31),
        ] {
            assert!(ContextId::parse(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn serde_roundtrip_is_a_plain_string() {
        let id = ContextId::from_content("file", b"hello");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        let back: ContextId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }
}
