//! The tools CtxC offers an agent.
//!
//! Each one is a thin translation: read the arguments, call the same code the
//! CLI calls, render the answer as text. Nothing here decides policy, and
//! nothing in the engine knows this file exists — which is what keeps MCP an
//! integration layer rather than a second front door with its own behaviour.
//!
//! Descriptions matter more than usual. A model reads them to decide whether a
//! tool is worth calling, so they say what the tool is *for* and what it costs,
//! not just what it does.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use ctxc_core::{Context, ContextId, ContextSource, HeuristicTokenizer, TokenBudget};
use ctxc_engine::index::{self, IndexOptions, Indexer};
use ctxc_engine::Engine;
use ctxc_project::{ProjectStore, Registry};
use ctxc_retrieval::{Query, RetrievalOptions, Retriever};
use ctxc_store::{
    ContextStore, Database, IndexStore, MemoryStore, SqliteContextStore, SqliteIndexStore,
    SqliteMemoryStore, SqliteProjectStore,
};

use crate::protocol::{schema, ToolDefinition, ToolResult};

/// Results returned by a search that does not compile.
const DEFAULT_LIMIT: usize = 10;

/// Notes listed when the agent does not say.
const DEFAULT_MEMORY_LIMIT: usize = 50;

/// Everything CtxC advertises over MCP.
pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "ctxc_search",
            description: "\
Find the code in a project most relevant to a question. Searches full text, \
symbol definitions, and the dependency graph together, then ranks the results. \
With compile=true it returns the content of the best files inside a token \
budget instead of a list — use that when you are about to read them anyway. \
Requires the project to have been indexed.",
            input_schema: schema(
                json!({
                    "query": {
                        "type": "string",
                        "description": "What to look for, in plain words. Quote a phrase to keep it together.",
                    },
                    "project": {
                        "type": "string",
                        "description": "Project id, path, or name. Defaults to the working directory.",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "How many results to return. Default 10.",
                        "minimum": 1,
                    },
                    "compile": {
                        "type": "boolean",
                        "description": "Return the content of the best files rather than a list.",
                    },
                    "budget": {
                        "type": "integer",
                        "description": "Token budget for compile. Defaults to the configured budget.",
                        "minimum": 1,
                    },
                }),
                &["query"],
            ),
        },
        ToolDefinition {
            name: "ctxc_optimize",
            description: "\
Shrink noisy text — build logs, test output, diffs, JSON — while keeping what \
matters. Returns the optimized text and a ctxc://context/<id> reference that \
ctxc_retrieve turns back into the original. Optimization is lossy on purpose; \
the reference is how you recover anything it removed.",
            input_schema: schema(
                json!({
                    "content": {
                        "type": "string",
                        "description": "The text to shrink.",
                    },
                    "source": {
                        "type": "string",
                        "description": "Where it came from, such as the command that produced it. Improves routing.",
                    },
                    "budget": {
                        "type": "integer",
                        "description": "Token budget for the result. Defaults to the configured budget.",
                        "minimum": 1,
                    },
                }),
                &["content"],
            ),
        },
        ToolDefinition {
            name: "ctxc_compile",
            description: "\
Combine several files into one AI-ready document, optimized and kept inside a \
token budget. Use it when you know which files you need; use ctxc_search when \
you do not.",
            input_schema: schema(
                json!({
                    "paths": {
                        "type": "array",
                        "description": "Files to include, in the order they should appear.",
                        "items": { "type": "string" },
                        "minItems": 1,
                    },
                    "budget": {
                        "type": "integer",
                        "description": "Token budget for the document. Defaults to the configured budget.",
                        "minimum": 1,
                    },
                }),
                &["paths"],
            ),
        },
        ToolDefinition {
            name: "ctxc_retrieve",
            description: "\
Recover the original text behind a ctxc://context/<id> reference. Every \
optimized output carries one. Use this instead of guessing at what optimization \
removed.",
            input_schema: schema(
                json!({
                    "reference": {
                        "type": "string",
                        "description": "The ctxc://context/<id> reference, or just the id.",
                    },
                }),
                &["reference"],
            ),
        },
        ToolDefinition {
            name: "ctxc_index",
            description: "\
Index a project so it can be searched: files, symbols, and how they depend on \
each other. Incremental — unchanged files are skipped — so running it again is \
cheap. Run it when a search returns nothing you expected.",
            input_schema: schema(
                json!({
                    "project": {
                        "type": "string",
                        "description": "Project id, path, or name. Defaults to the working directory.",
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Re-parse every file, even ones that look unchanged.",
                    },
                }),
                &[],
            ),
        },
        ToolDefinition {
            name: "ctxc_memory",
            description: "\
Keep short notes about a project across sessions — build quirks, decisions, \
where something lives. Actions: save, get, list, forget. Notes are stored \
locally and scoped to the project; they are not searched automatically, so give \
each one a key you will think to ask for.",
            input_schema: schema(
                json!({
                    "action": {
                        "type": "string",
                        "description": "What to do.",
                        "enum": ["save", "get", "list", "forget"],
                    },
                    "key": {
                        "type": "string",
                        "description": "Name of the note. Required for save, get, and forget.",
                    },
                    "value": {
                        "type": "string",
                        "description": "The note itself. Required for save.",
                    },
                    "project": {
                        "type": "string",
                        "description": "Project id, path, or name. Defaults to the working directory.",
                    },
                }),
                &["action"],
            ),
        },
    ]
}

/// What a tool needs in order to answer.
pub struct Tools {
    config: ctxc_core::Config,
    database_path: PathBuf,
    /// Where relative paths and an unnamed project resolve from.
    working_directory: PathBuf,
}

impl Tools {
    pub fn new(
        config: ctxc_core::Config,
        database_path: PathBuf,
        working_directory: PathBuf,
    ) -> Self {
        Tools {
            config,
            database_path,
            working_directory,
        }
    }

    /// Run one tool call.
    ///
    /// Failures come back as a failed [`ToolResult`] rather than an error: the
    /// model should see what went wrong and be able to act on it, and a
    /// transport-level error is invisible to it.
    pub fn call(&self, name: &str, arguments: &Value) -> ToolResult {
        let outcome = match name {
            "ctxc_search" => self.search(arguments),
            "ctxc_optimize" => self.optimize(arguments),
            "ctxc_compile" => self.compile(arguments),
            "ctxc_retrieve" => self.retrieve(arguments),
            "ctxc_index" => self.index(arguments),
            "ctxc_memory" => self.memory(arguments),
            other => return ToolResult::failed(format!("there is no tool called `{other}`")),
        };

        match outcome {
            Ok(result) => result,
            Err(message) => ToolResult::failed(message),
        }
    }

    fn database(&self) -> Result<Database, String> {
        Database::open(&self.database_path).map_err(|err| {
            format!(
                "could not open the CtxC database ({}): {err}",
                self.database_path.display()
            )
        })
    }

    /// Resolve the project an argument names, as a root key and a display name.
    ///
    /// A registered project can be named by id, path, or name. Anything else is
    /// treated as a directory, which is what makes these tools usable in a
    /// project nobody has registered.
    fn project(&self, database: &Database, named: Option<&str>) -> Result<Scope, String> {
        let store = SqliteProjectStore::new(database);

        if let Some(reference) = named {
            if let Ok(project) = Registry::new(&store).resolve(reference) {
                return Ok(Scope {
                    key: project.path.clone(),
                    root: PathBuf::from(&project.path),
                    name: project.name,
                    id: Some(project.id.as_str().to_owned()),
                });
            }
        }

        let root = match named {
            Some(path) => self.working_directory.join(path),
            None => self.working_directory.clone(),
        };
        if !root.is_dir() {
            return Err(format!(
                "`{}` is not a registered project and not a directory",
                named.unwrap_or(".")
            ));
        }

        let key = index::root_key(&root);
        let registered = ProjectStore::project_by_path(&store, &key).ok().flatten();

        Ok(Scope {
            name: registered
                .as_ref()
                .map(|project| project.name.clone())
                .unwrap_or_else(|| {
                    root.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| key.clone())
                }),
            id: registered.map(|project| project.id.as_str().to_owned()),
            key,
            root,
        })
    }

    fn budget(&self, arguments: &Value) -> TokenBudget {
        arguments
            .get("budget")
            .and_then(Value::as_u64)
            .map(|budget| TokenBudget::new(budget.min(u32::MAX as u64) as u32))
            .unwrap_or_else(|| self.config.default_budget())
    }

    // ------------------------------------------------------------ search

    fn search(&self, arguments: &Value) -> Result<ToolResult, String> {
        let query = string(arguments, "query")?;
        let database = self.database()?;
        let scope = self.project(&database, optional(arguments, "project"))?;

        let store = SqliteIndexStore::new(&database);
        let counts = store
            .counts(&scope.key)
            .map_err(|err| format!("could not read the index: {err}"))?;
        if counts.files == 0 {
            return Ok(ToolResult::failed(format!(
                "`{}` has not been indexed yet. Run the ctxc_index tool first.",
                scope.name
            )));
        }

        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_LIMIT);

        let options = RetrievalOptions {
            limit,
            ..RetrievalOptions::from_config(&self.config)
        };
        let retrieval = Retriever::new(&store, options)
            .retrieve(
                &scope.key,
                &Query::parse(&query),
                ctxc_core::Timestamp::now().as_millis(),
            )
            .map_err(|err| format!("search failed: {err}"))?;

        if retrieval.files.is_empty() {
            return Ok(ToolResult::ok(format!(
                "Nothing in `{}` matches {query:?}.\n\n\
                 The index may be stale — the ctxc_index tool refreshes it.",
                scope.name
            )));
        }

        if !arguments
            .get("compile")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(ToolResult::ok(render_results(&retrieval, &scope.name)));
        }

        // Compiling reads content through the index rather than the
        // filesystem, so what comes back is exactly what was searched.
        let tokenizer = HeuristicTokenizer::new();
        let budget = self.budget(arguments);
        let selected = retrieval.select_within_budget(budget, &tokenizer, |path| {
            store.file_content(&scope.key, path).ok().flatten()
        });

        let mut contexts = Vec::with_capacity(selected.len());
        for path in &selected {
            let Some(content) = store
                .file_content(&scope.key, path)
                .map_err(|err| format!("could not read {path}: {err}"))?
            else {
                continue;
            };
            contexts.push(Context::new(
                ContextSource::File {
                    path: PathBuf::from(path),
                },
                ctxc_core::ContentType::Code,
                content,
            ));
        }

        if contexts.is_empty() {
            return Ok(ToolResult::ok(format!(
                "{} files matched, but none of their content is in the index.\n\n\
                 Run the ctxc_index tool to refresh it.",
                retrieval.files.len()
            )));
        }

        let compilation = Engine::from_config(&self.config)
            .compile(&contexts, Some(budget))
            .map_err(|err| format!("could not compile the results: {err}"))?;

        Ok(ToolResult::ok(format!(
            "{}\n\n---\n{} file(s), {} tokens (estimated), {:.0}% smaller than the originals.",
            compilation.content,
            compilation.sections.len(),
            compilation.optimized_tokens,
            compilation.reduction_ratio * 100.0,
        )))
    }

    // ------------------------------------------------------------ optimize

    fn optimize(&self, arguments: &Value) -> Result<ToolResult, String> {
        let content = string(arguments, "content")?;
        let source = match optional(arguments, "source") {
            Some(command) => ContextSource::Command {
                command: command.to_owned(),
            },
            None => ContextSource::Api {
                endpoint: "mcp/ctxc_optimize".into(),
            },
        };

        let context = ctxc_context::ingest::from_text(source, &content, None);
        let optimized = Engine::from_config(&self.config)
            .optimize(&context, Some(self.budget(arguments)))
            .map_err(|err| format!("could not optimize that: {err}"))?;

        // The original is stored, because the reference below is a promise
        // that it can be read back.
        let database = self.database()?;
        SqliteContextStore::new(&database)
            .save_context(&context)
            .map_err(|err| format!("could not store the original: {err}"))?;

        let result = &optimized.result;
        Ok(ToolResult::ok(format!(
            "{}\n\n---\n{} -> {} tokens (estimated), {:.0}% smaller.\nOriginal: {}",
            optimized.content,
            result.original_tokens,
            result.optimized_tokens,
            result.reduction_ratio * 100.0,
            optimized.reference(),
        )))
    }

    // ------------------------------------------------------------ compile

    fn compile(&self, arguments: &Value) -> Result<ToolResult, String> {
        let paths = arguments
            .get("paths")
            .and_then(Value::as_array)
            .filter(|paths| !paths.is_empty())
            .ok_or("`paths` must be a non-empty array of file paths")?;

        let mut contexts = Vec::with_capacity(paths.len());
        for entry in paths {
            let path = entry
                .as_str()
                .ok_or("every entry in `paths` must be a string")?;
            let resolved = self.working_directory.join(path);

            let content = std::fs::read_to_string(&resolved)
                .map_err(|err| format!("could not read {}: {err}", resolved.display()))?;
            contexts.push(ctxc_context::ingest::from_text(
                ContextSource::File {
                    path: resolved.clone(),
                },
                &content,
                Some(&resolved),
            ));
        }

        let compilation = Engine::from_config(&self.config)
            .compile(&contexts, Some(self.budget(arguments)))
            .map_err(|err| format!("could not compile those files: {err}"))?;

        let database = self.database()?;
        let store = SqliteContextStore::new(&database);
        for context in &contexts {
            // A storage failure costs the reference, not the answer.
            if let Err(err) = store.save_context(context) {
                tracing::debug!(error = %err, "could not store a compiled input");
            }
        }

        Ok(ToolResult::ok(format!(
            "{}\n\n---\n{} file(s), {} -> {} tokens (estimated), {:.0}% smaller.",
            compilation.content,
            compilation.sections.len(),
            compilation.original_tokens,
            compilation.optimized_tokens,
            compilation.reduction_ratio * 100.0,
        )))
    }

    // ------------------------------------------------------------ retrieve

    fn retrieve(&self, arguments: &Value) -> Result<ToolResult, String> {
        let reference = string(arguments, "reference")?;
        // The parse error already names the reference, so wrapping it would
        // say the same thing twice.
        let id = ContextId::parse(&reference).map_err(|err| err.to_string())?;

        let database = self.database()?;
        let context = SqliteContextStore::new(&database)
            .get_context(&id)
            .map_err(|err| format!("could not read the context database: {err}"))?;

        match context {
            Some(context) => Ok(ToolResult::ok(context.content)),
            None => Ok(ToolResult::failed(format!(
                "nothing is stored for {}. References last as long as the \
                 database does; check the id, or re-run whatever produced it.",
                id.to_uri()
            ))),
        }
    }

    // ------------------------------------------------------------ index

    fn index(&self, arguments: &Value) -> Result<ToolResult, String> {
        let database = self.database()?;
        let scope = self.project(&database, optional(arguments, "project"))?;

        let options = IndexOptions {
            force: arguments
                .get("force")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ..IndexOptions::default()
        };

        let store = SqliteIndexStore::new(&database);
        let report = database
            .transaction(|| Indexer::new(&store).index(&scope.root, &options))
            .map_err(|err: ctxc_engine::EngineError| {
                format!("could not index {}: {err}", scope.root.display())
            })?;

        Ok(ToolResult::ok(format!(
            "Indexed `{}`.\n\n\
             Scanned {} file(s), indexed {}, unchanged {}, removed {}.\n\
             {} symbol(s), {} relationship(s), in {} ms.",
            scope.name,
            report.scanned,
            report.indexed,
            report.unchanged,
            report.removed,
            report.symbols,
            report.relationships,
            report.duration_ms,
        )))
    }

    // ------------------------------------------------------------ memory

    fn memory(&self, arguments: &Value) -> Result<ToolResult, String> {
        let action = string(arguments, "action")?;
        let database = self.database()?;

        // A note about a directory that is not a project still belongs
        // somewhere, so an unregistered project scopes to nothing rather than
        // failing outright.
        let scope = self.project(&database, optional(arguments, "project")).ok();
        let project_id = scope.as_ref().and_then(|scope| scope.id.as_deref());
        let store = SqliteMemoryStore::new(&database);

        match action.as_str() {
            "save" => {
                let key = string(arguments, "key")?;
                let value = string(arguments, "value")?;
                let saved = store
                    .remember(project_id, &key, &value)
                    .map_err(|err| format!("could not save that note: {err}"))?;

                Ok(ToolResult::ok(format!(
                    "Saved `{}`{}.",
                    saved.key,
                    scope
                        .as_ref()
                        .map(|scope| format!(" for `{}`", scope.name))
                        .unwrap_or_default()
                )))
            }
            "get" => {
                let key = string(arguments, "key")?;
                match store
                    .recall(project_id, &key)
                    .map_err(|err| format!("could not read that note: {err}"))?
                {
                    Some(memory) => Ok(ToolResult::ok(memory.value)),
                    None => Ok(ToolResult::failed(format!("no note called `{key}`"))),
                }
            }
            "list" => {
                let memories = store
                    .memories(project_id, DEFAULT_MEMORY_LIMIT)
                    .map_err(|err| format!("could not list notes: {err}"))?;

                if memories.is_empty() {
                    return Ok(ToolResult::ok(
                        "No notes yet. Save one with action=save.".to_string(),
                    ));
                }

                let listed: Vec<String> = memories
                    .iter()
                    .map(|memory| format!("- {}: {}", memory.key, summarize(&memory.value)))
                    .collect();
                Ok(ToolResult::ok(listed.join("\n")))
            }
            "forget" => {
                let key = string(arguments, "key")?;
                let removed = store
                    .forget(project_id, &key)
                    .map_err(|err| format!("could not forget that note: {err}"))?;

                Ok(ToolResult::ok(if removed {
                    format!("Forgot `{key}`.")
                } else {
                    format!("There was no note called `{key}`.")
                }))
            }
            other => Ok(ToolResult::failed(format!(
                "`{other}` is not an action; use save, get, list, or forget"
            ))),
        }
    }
}

/// A resolved project: what to search, and what to call it.
struct Scope {
    /// Canonical root key, the form the index uses.
    key: String,
    root: PathBuf,
    name: String,
    /// Registry id, when the project is registered.
    id: Option<String>,
}

/// A required string argument.
fn string(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("`{name}` is required"))
}

/// An optional string argument.
fn optional<'a>(arguments: &'a Value, name: &str) -> Option<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

/// A note's first line, short enough for a list.
fn summarize(value: &str) -> String {
    let first = value.lines().next().unwrap_or_default().trim();
    if first.chars().count() <= 80 {
        return first.to_owned();
    }
    let kept: String = first.chars().take(79).collect();
    format!("{kept}…")
}

/// Ranked results, as prose rather than JSON.
///
/// A model reads this; it does not parse it. Saying *why* each file matched is
/// what lets it decide which ones to open.
fn render_results(retrieval: &ctxc_retrieval::Retrieval, project: &str) -> String {
    let mut out = format!(
        "{} result(s) in `{project}` for {:?}:\n\n",
        retrieval.files.len(),
        retrieval.query
    );

    for file in &retrieval.files {
        let location = match file.line {
            Some(line) => format!("{}:{}", file.path, line),
            None => file.path.clone(),
        };
        out.push_str(&format!("{location}  (score {:.2})\n", file.score));

        if !file.matched_symbols.is_empty() {
            out.push_str(&format!("  defines {}\n", file.matched_symbols.join(", ")));
        }
        if let ctxc_retrieval::Reason::Related { to, hops } = &file.reason {
            out.push_str(&format!("  related to {to} ({hops} hop away)\n"));
        }
        if let Some(snippet) = &file.snippet {
            for line in snippet.lines() {
                out.push_str(&format!("  | {line}\n"));
            }
        }
        out.push('\n');
    }

    out.push_str("Call this tool again with compile=true to get the content of these files.");
    out
}

/// The path a tool should treat as the working directory.
pub fn working_directory() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_a_client_can_read() {
        for tool in definitions() {
            assert!(tool.name.starts_with("ctxc_"), "{}", tool.name);
            assert!(
                tool.description.len() > 40,
                "{} needs a description a model can act on",
                tool.name
            );
            assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
            assert!(tool.input_schema["properties"].is_object(), "{}", tool.name);
        }
    }

    #[test]
    fn the_five_phase_tools_are_all_present() {
        let names: Vec<&str> = definitions().into_iter().map(|tool| tool.name).collect();

        for expected in [
            "ctxc_search",
            "ctxc_retrieve",
            "ctxc_compile",
            "ctxc_optimize",
            "ctxc_index",
            "ctxc_memory",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} is missing: {names:?}"
            );
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let names: Vec<&str> = definitions().into_iter().map(|tool| tool.name).collect();
        let unique: std::collections::HashSet<&&str> = names.iter().collect();

        assert_eq!(unique.len(), names.len(), "{names:?}");
    }

    #[test]
    fn required_arguments_are_reported_by_name() {
        let arguments = json!({ "query": "  " });

        assert_eq!(
            string(&arguments, "query").unwrap_err(),
            "`query` is required",
            "whitespace is not an argument"
        );
        assert_eq!(
            string(&arguments, "missing").unwrap_err(),
            "`missing` is required"
        );
        assert_eq!(string(&json!({"q": "x"}), "q").unwrap(), "x");
    }

    #[test]
    fn optional_arguments_treat_blank_as_absent() {
        assert_eq!(
            optional(&json!({"project": "acme"}), "project"),
            Some("acme")
        );
        assert_eq!(optional(&json!({"project": "   "}), "project"), None);
        assert_eq!(optional(&json!({}), "project"), None);
    }

    #[test]
    fn a_note_summary_is_one_short_line() {
        assert_eq!(summarize("short note"), "short note");
        assert_eq!(summarize("first line\nsecond line"), "first line");
        assert_eq!(summarize(&"x".repeat(200)).chars().count(), 80);
    }

    #[test]
    fn an_unknown_tool_is_a_failed_result_not_a_panic() {
        let tools = Tools::new(
            ctxc_core::Config::default(),
            PathBuf::from(":memory:"),
            PathBuf::from("."),
        );

        let result = tools.call("ctxc_teleport", &json!({}));
        assert!(result.is_error);
        assert!(result.text.contains("ctxc_teleport"), "{}", result.text);
    }
}
