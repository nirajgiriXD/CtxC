//! Editing one key in someone else's JSON.
//!
//! MCP clients are configured with a JSON file listing servers, and CtxC needs
//! exactly one entry in it. There is no comment syntax to hide a marker in, so
//! the managed region is a key: CtxC owns `mcpServers.ctxc` and nothing else.
//!
//! ```text
//! {
//!   "mcpServers": {
//!     "other-tool": { ... },   <- theirs, untouched
//!     "ctxc":       { ... }    <- ours
//!   }
//! }
//! ```
//!
//! A file that is not valid JSON is refused rather than replaced. `.vscode`
//! configuration in particular is often JSONC — comments and trailing commas —
//! and parsing that as JSON then writing it back would silently delete the
//! comments. Better to say so and let a person decide.

use serde_json::{Map, Value};

use crate::block::Change;

/// A server entry CtxC manages inside a JSON configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedEntry {
    /// The object servers are listed under: `mcpServers` for most clients.
    pub section: &'static str,
    /// The key CtxC owns inside it.
    pub key: &'static str,
}

impl ManagedEntry {
    pub const fn new(section: &'static str, key: &'static str) -> Self {
        ManagedEntry { section, key }
    }

    /// Read a document, treating an empty file as an empty object.
    ///
    /// Returns `None` when the text is not JSON, which the caller must report
    /// rather than paper over.
    pub fn parse(&self, document: &str) -> Option<Value> {
        if document.trim().is_empty() {
            return Some(Value::Object(Map::new()));
        }
        serde_json::from_str(document).ok()
    }

    /// The entry currently stored, if there is one.
    pub fn read(&self, document: &Value) -> Option<Value> {
        document.get(self.section)?.get(self.key).cloned()
    }

    /// Put `entry` in the document under this key.
    ///
    /// Every other key, in this section and outside it, is preserved. Returns
    /// the new document and what changed.
    pub fn write(&self, document: &Value, entry: Value) -> (Value, Change) {
        let mut root = match document {
            Value::Object(map) => map.clone(),
            // A configuration file holding an array or a string is not
            // something to merge into; replace it and let the caller's
            // reporting say a file was rewritten.
            _ => Map::new(),
        };

        let mut section = match root.get(self.section) {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        };

        let change = match section.get(self.key) {
            Some(existing) if *existing == entry => Change::Unchanged,
            Some(_) => Change::Updated,
            None => Change::Added,
        };
        if change == Change::Unchanged {
            return (document.clone(), change);
        }

        section.insert(self.key.to_string(), entry);
        root.insert(self.section.to_string(), Value::Object(section));
        (Value::Object(root), change)
    }

    /// Take CtxC's key out.
    ///
    /// Returns `None` when it was not there. An emptied section goes too,
    /// rather than being left behind as `{"mcpServers": {}}`.
    pub fn remove(&self, document: &Value) -> Option<Value> {
        let Value::Object(root) = document else {
            return None;
        };
        let Some(Value::Object(section)) = root.get(self.section) else {
            return None;
        };
        if !section.contains_key(self.key) {
            return None;
        }

        let mut root = root.clone();
        let mut section = section.clone();
        section.remove(self.key);

        if section.is_empty() {
            root.remove(self.section);
        } else {
            root.insert(self.section.to_string(), Value::Object(section));
        }
        Some(Value::Object(root))
    }

    /// Whether removing our key would leave nothing behind.
    pub fn is_now_empty(&self, document: &Value) -> bool {
        matches!(document, Value::Object(map) if map.is_empty())
    }
}

/// Render a document the way a person would have written it.
///
/// Two-space indentation and a trailing newline: this file is going into
/// somebody's repository, and a one-line blob would show up in every diff.
pub fn render(document: &Value) -> String {
    let mut text = serde_json::to_string_pretty(document)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("\r\n", "\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ENTRY: ManagedEntry = ManagedEntry::new("mcpServers", "ctxc");

    fn ctxc() -> Value {
        json!({ "command": "ctxc", "args": ["mcp"] })
    }

    #[test]
    fn an_empty_file_becomes_a_document_with_one_server() {
        let document = ENTRY.parse("").unwrap();
        let (written, change) = ENTRY.write(&document, ctxc());

        assert_eq!(change, Change::Added);
        assert_eq!(written["mcpServers"]["ctxc"]["command"], "ctxc");
    }

    #[test]
    fn other_servers_are_left_exactly_as_they_were() {
        let document = ENTRY
            .parse(r#"{"mcpServers":{"other":{"command":"other-tool","args":["--serve"]}}}"#)
            .unwrap();

        let (written, change) = ENTRY.write(&document, ctxc());
        assert_eq!(change, Change::Added);
        assert_eq!(written["mcpServers"]["other"]["command"], "other-tool");
        assert_eq!(written["mcpServers"]["other"]["args"][0], "--serve");
        assert_eq!(written["mcpServers"]["ctxc"]["command"], "ctxc");
    }

    #[test]
    fn keys_outside_the_section_survive() {
        let document = ENTRY
            .parse(r#"{"someOtherSetting":true,"mcpServers":{}}"#)
            .unwrap();

        let (written, _) = ENTRY.write(&document, ctxc());
        assert_eq!(written["someOtherSetting"], true);
    }

    #[test]
    fn writing_the_same_entry_twice_changes_nothing() {
        let document = ENTRY.parse("").unwrap();
        let (once, _) = ENTRY.write(&document, ctxc());
        let (twice, change) = ENTRY.write(&once, ctxc());

        assert_eq!(change, Change::Unchanged);
        assert_eq!(once, twice);
    }

    #[test]
    fn a_changed_entry_is_an_update() {
        let document = ENTRY
            .parse(r#"{"mcpServers":{"ctxc":{"command":"old-path/ctxc","args":[]}}}"#)
            .unwrap();

        let (written, change) = ENTRY.write(&document, ctxc());
        assert_eq!(change, Change::Updated);
        assert_eq!(written["mcpServers"]["ctxc"]["command"], "ctxc");
    }

    #[test]
    fn removing_takes_our_key_and_leaves_the_others() {
        let document = ENTRY
            .parse(r#"{"mcpServers":{"other":{"command":"x"},"ctxc":{"command":"ctxc"}}}"#)
            .unwrap();

        let cleaned = ENTRY.remove(&document).unwrap();
        assert!(cleaned["mcpServers"].get("ctxc").is_none());
        assert_eq!(cleaned["mcpServers"]["other"]["command"], "x");
    }

    #[test]
    fn an_emptied_section_goes_rather_than_being_left_behind() {
        let document = ENTRY
            .parse(r#"{"mcpServers":{"ctxc":{"command":"ctxc"}}}"#)
            .unwrap();

        let cleaned = ENTRY.remove(&document).unwrap();
        assert!(cleaned.get("mcpServers").is_none(), "{cleaned}");
        assert!(
            ENTRY.is_now_empty(&cleaned),
            "a file CtxC filled entirely can be deleted: {cleaned}"
        );
    }

    #[test]
    fn a_file_with_other_settings_is_not_reported_as_empty() {
        let document = ENTRY
            .parse(r#"{"somethingElse":1,"mcpServers":{"ctxc":{"command":"ctxc"}}}"#)
            .unwrap();

        let cleaned = ENTRY.remove(&document).unwrap();
        assert!(!ENTRY.is_now_empty(&cleaned));
    }

    #[test]
    fn removing_what_was_never_there_reports_nothing_to_do() {
        assert!(ENTRY.remove(&json!({})).is_none());
        assert!(ENTRY
            .remove(&json!({"mcpServers": {"other": {}}}))
            .is_none());
        assert!(ENTRY.remove(&json!("not an object")).is_none());
    }

    #[test]
    fn a_file_that_is_not_json_is_refused_rather_than_replaced() {
        // JSONC, which `.vscode` configuration often is. Parsing this as JSON
        // and writing it back would delete the comment.
        assert!(ENTRY
            .parse("{\n  // the tools I use\n  \"mcpServers\": {}\n}")
            .is_none());
        assert!(ENTRY.parse("not json at all").is_none());
    }

    #[test]
    fn rendering_is_something_a_person_would_have_typed() {
        let (written, _) = ENTRY.write(&ENTRY.parse("").unwrap(), ctxc());
        let text = render(&written);

        assert!(text.starts_with("{\n  \"mcpServers\""), "{text}");
        assert!(text.ends_with("}\n"), "{text:?}");
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            written,
            "what is written back must parse to what was meant"
        );
    }
}
