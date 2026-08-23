//! Routes that let a client run CtxC rather than only read it.
//!
//! Configuration, the command catalog, stored context, the dependency graph,
//! diagnostics and logs. They live beside the rest of the API rather than in a
//! privileged corner: the dashboard is the main caller, but nothing here is
//! reserved for it, and `ctxc` can reach every one of them.
//!
//! Handlers stay thin, as everywhere else in this crate. Editing a
//! configuration file is [`ctxc_core::config_file`]'s job; deciding what a
//! dependency graph means is [`ctxc_graph`]'s. What is left here is translating
//! a request into a call and an answer back.

use axum::extract::{Path as PathParam, Query as QueryParams, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use ctxc_core::config::{Config, LayerInfo, LoadOptions, PartialConfig};
use ctxc_core::config_file::{self, Change};
use ctxc_core::platform::SystemEnvironment;
use ctxc_core::ContextId;
use ctxc_graph::DependencyGraph;
use ctxc_metrics::{MetricEvent, Operation};
use ctxc_project::Registry;
use ctxc_store::{
    ContextStore, IndexStore, SqliteContextStore, SqliteIndexStore, SqliteProjectStore,
};

use crate::logs::{self, LogPage, Severity};
use crate::routes::{ApiResult, Failure};
use crate::state::{ApiState, Locations};

// ------------------------------------------------------------- configuration

/// The configuration, and everything needed to edit it honestly.
///
/// Serialize only: [`Config`] is built by merging layers rather than read whole
/// from a document, so there is no meaningful way to deserialize one back.
#[derive(Debug, Serialize)]
pub struct ConfigView {
    /// What the daemon is running with, as it was loaded at startup.
    pub running: Config,
    /// What the layers say now. The two differ after an edit, until a restart.
    pub effective: Config,
    /// The effective configuration as a file would spell it.
    pub toml: String,
    pub file: ConfigFileView,
    /// Keys set by `CTXC_*` variables. These win over the file, so a settings
    /// screen must show them as fixed rather than offer to change them.
    pub environment_keys: Vec<String>,
    /// Every key the file layer can hold.
    pub editable_keys: Vec<&'static str>,
    pub layers: Vec<LayerInfo>,
    pub locations: Locations,
    /// True when the daemon is running with something other than what the
    /// layers now say.
    pub restart_required: bool,
}

/// The editable layer.
#[derive(Debug, Serialize)]
pub struct ConfigFileView {
    pub path: String,
    pub exists: bool,
    /// The keys this file decides, dotted.
    pub keys: Vec<&'static str>,
    /// The file layer as TOML — what is actually written down, not the
    /// defaults it sits on.
    pub toml: String,
}

/// An edit: values to write, and keys to hand back to the defaults.
#[derive(Debug, Default, Deserialize)]
pub struct ConfigEdit {
    #[serde(default)]
    pub set: PartialConfig,
    #[serde(default)]
    pub reset: Vec<String>,
}

/// What an edit did, and what the configuration looks like afterwards.
#[derive(Debug, Serialize)]
pub struct ConfigEdited {
    pub changed: Vec<String>,
    /// Keys the edit wrote that an environment variable still overrides.
    pub shadowed: Vec<String>,
    pub created: bool,
    #[serde(flatten)]
    pub config: ConfigView,
}

pub(crate) async fn config(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<ConfigView> {
    crate::routes::authorize(&state, &headers)?;
    Ok(axum::Json(read_config(&state)?))
}

pub(crate) async fn edit_config(
    State(state): State<ApiState>,
    headers: HeaderMap,
    axum::Json(edit): axum::Json<ConfigEdit>,
) -> ApiResult<ConfigEdited> {
    crate::routes::authorize(&state, &headers)?;

    let change = Change {
        set: edit.set,
        reset: edit.reset,
    };

    // An edit that asks for nothing is answered rather than refused: a settings
    // form that saves an unchanged section should not look like a failure.
    let applied = config_file::apply(&state.locations().config_file, change, &SystemEnvironment)?;

    if !applied.changed.is_empty() {
        state.publish(crate::events::StreamEvent::Config {
            changed: applied.changed.clone(),
        });
    }

    Ok(axum::Json(ConfigEdited {
        changed: applied.changed,
        shadowed: applied.shadowed,
        created: applied.created,
        config: read_config(&state)?,
    }))
}

/// Read every layer again and describe the result.
fn read_config(state: &ApiState) -> Result<ConfigView, Failure> {
    let locations = state.locations().clone();
    let env = SystemEnvironment;

    let loaded = Config::load(LoadOptions::new(&env).with_file(&locations.config_file))?;
    let file_layer = PartialConfig::from_file(&locations.config_file)?.unwrap_or_default();
    let environment = PartialConfig::from_env(&env)?;

    let running = state.config().clone();
    let restart_required = loaded.config != running;

    Ok(ConfigView {
        toml: loaded.config.to_toml(),
        file: ConfigFileView {
            path: locations.config_file.display().to_string(),
            exists: locations.config_file.exists(),
            keys: file_layer.set_keys(),
            toml: file_layer.to_toml(),
        },
        environment_keys: environment
            .set_keys()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        editable_keys: config_file::keys(),
        layers: loaded.layers,
        effective: loaded.config,
        running,
        locations,
        restart_required,
    })
}

// ----------------------------------------------------------------- commands

/// The command tree this build has, or an explanation of why it has none.
pub(crate) async fn commands(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<crate::commands::Catalog> {
    crate::routes::authorize(&state, &headers)?;

    match crate::commands::catalog() {
        Some(catalog) => Ok(axum::Json(catalog.clone())),
        // Only reachable when something other than the `ctxc` binary is hosting
        // the API. Saying so beats returning an empty list that reads as "this
        // build can do nothing".
        None => Err(Failure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "this process did not publish a command catalog",
        )
        .with_hint(Some(
            "the catalog comes from the `ctxc` binary; run the daemon with `ctxc start`".into(),
        ))),
    }
}

// ------------------------------------------------------------ stored context

/// Content recovered from a `ctxc://context/<id>` reference.
#[derive(Debug, Serialize, Deserialize)]
pub struct StoredContext {
    pub id: String,
    pub reference: String,
    pub source: String,
    pub content_type: String,
    pub bytes: u64,
    pub content: String,
}

pub(crate) async fn context(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(reference): PathParam<String>,
) -> ApiResult<StoredContext> {
    crate::routes::authorize(&state, &headers)?;

    let id = ContextId::parse(&reference).map_err(|_| {
        Failure::new(
            StatusCode::BAD_REQUEST,
            format!("{reference} is not a context reference"),
        )
        .with_hint(Some(
            "references look like `ctxc://context/<id>`, and the id alone works too".into(),
        ))
    })?;

    state.with_database(|database| {
        let store = SqliteContextStore::new(database);
        let context = store.get_context(&id)?.ok_or_else(|| {
            Failure::new(
                StatusCode::NOT_FOUND,
                format!("no context is stored for {}", id.to_uri()),
            )
            .with_hint(Some(
                "originals are kept unless the command ran with --no-store; \
                 check the id, or re-run what produced it"
                    .into(),
            ))
        })?;

        state.record(MetricEvent::new(Operation::Retrieve, id.to_string()).with_cache_hit(true));

        Ok(axum::Json(StoredContext {
            id: id.to_string(),
            reference: context.uri(),
            source: ctxc_context::ingest::label(&context.metadata.source),
            content_type: context.metadata.content_type.as_str().to_string(),
            bytes: context.metadata.byte_len,
            content: context.content,
        }))
    })
}

// ------------------------------------------------------------ indexed files

/// Which file to read, relative to the project root.
#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
}

/// One file as the index holds it.
///
/// Symbols are not here: the index stores them for search, and there is no
/// per-file read for them that would not be a new query written for one screen.
#[derive(Debug, Serialize, Deserialize)]
pub struct IndexedFileView {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub content: String,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
}

pub(crate) async fn project_file(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
    QueryParams(query): QueryParams<FileQuery>,
) -> ApiResult<IndexedFileView> {
    crate::routes::authorize(&state, &headers)?;

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).resolve(&id)?;
        let store = SqliteIndexStore::new(database);

        // The index is the source, not the filesystem. Serving whatever is on
        // disk would let any path reachable from the daemon be read through a
        // query string; serving what was indexed cannot.
        let content = store
            .file_content(&project.path, &query.path)?
            .ok_or_else(|| {
                Failure::new(
                    StatusCode::NOT_FOUND,
                    format!("{} is not in the index", query.path),
                )
                .with_hint(Some(
                    "re-index the project, or check the path is relative to its root".into(),
                ))
            })?;

        let file = store.file(&project.path, &query.path)?;
        let graph = DependencyGraph::from_edges(store.edges(&project.path)?);

        Ok(axum::Json(IndexedFileView {
            language: file.and_then(|file| file.language),
            dependencies: graph
                .dependencies_of(&query.path)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            dependents: graph
                .dependents_of(&query.path)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            path: query.path,
            content,
        }))
    })
}

// ----------------------------------------------------------- dependency graph

/// What to ask the graph.
#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    /// Look at one file instead of the whole project.
    #[serde(default)]
    pub file: Option<String>,
    /// How many files to list in the summary.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// How a project's files depend on each other.
#[derive(Debug, Serialize, Deserialize)]
pub struct GraphView {
    pub files: usize,
    pub edges: usize,
    /// The files the rest of the project leans on most.
    pub most_depended_on: Vec<GraphEntry>,
    /// Set when the request named a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileGraphView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphEntry {
    pub path: String,
    pub dependents: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileGraphView {
    pub path: String,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
}

/// Files listed in a graph summary when the caller does not say.
const DEFAULT_GRAPH_LIMIT: usize = 10;

/// The most a summary will list, so a query string cannot ask for the whole
/// project one row at a time.
const MAX_GRAPH_LIMIT: usize = 200;

pub(crate) async fn project_graph(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
    QueryParams(query): QueryParams<GraphQuery>,
) -> ApiResult<GraphView> {
    crate::routes::authorize(&state, &headers)?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_GRAPH_LIMIT)
        .clamp(1, MAX_GRAPH_LIMIT);

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).resolve(&id)?;
        let store = SqliteIndexStore::new(database);
        let graph = DependencyGraph::from_edges(store.edges(&project.path)?);

        Ok(axum::Json(GraphView {
            files: graph.node_count(),
            edges: graph.edge_count(),
            most_depended_on: graph
                .most_depended_on(limit)
                .into_iter()
                .map(|(path, dependents)| GraphEntry {
                    path: path.to_owned(),
                    dependents,
                })
                .collect(),
            file: query.file.map(|path| FileGraphView {
                dependencies: graph
                    .dependencies_of(&path)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                dependents: graph
                    .dependents_of(&path)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                path,
            }),
        }))
    })
}

// --------------------------------------------------------------- diagnostics

/// What this installation looks like, for the panel that answers "is anything
/// wrong?".
///
/// The same facts `ctxc status` prints, from the same places — a person
/// comparing the two should never have to wonder which one is right.
///
/// Serialize only: `routes` borrows the router's own list rather than copying
/// it, which is the right trade for a report nothing reads back.
#[derive(Debug, Serialize)]
pub struct Diagnostics {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub pid: u32,
    pub uptime_ms: u64,
    pub started_at: i64,
    pub locations: Locations,
    pub config_file_exists: bool,
    pub database: DatabaseView,
    /// Roots the index has seen, which can outlast a project's registration.
    pub indexed_roots: usize,
    pub contexts: u64,
    /// The routes this build answers.
    pub routes: Vec<&'static str>,
    pub logs: LogCapacity,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DatabaseView {
    pub path: String,
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogCapacity {
    pub held: usize,
    pub dropped: u64,
    pub capacity: usize,
}

pub(crate) async fn diagnostics(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Diagnostics> {
    crate::routes::authorize(&state, &headers)?;
    let locations = state.locations().clone();
    let page = logs::buffer().page(0, None, None);

    state.with_database(|database| {
        let contexts = SqliteContextStore::new(database).count_contexts()?;
        let indexed_roots = SqliteIndexStore::new(database).roots()?.len();

        Ok(axum::Json(Diagnostics {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: ctxc_core::Os::current().as_str().to_string(),
            arch: std::env::consts::ARCH.to_string(),
            pid: std::process::id(),
            uptime_ms: state.uptime_ms(),
            started_at: state.started_at().as_millis(),
            config_file_exists: locations.config_file.exists(),
            database: DatabaseView {
                path: locations.database.display().to_string(),
                schema_version: database.schema_version()?,
                size_bytes: database.size_on_disk(),
            },
            indexed_roots,
            contexts,
            routes: crate::routes::routes(),
            logs: LogCapacity {
                held: logs::buffer().len(),
                dropped: page.dropped,
                capacity: page.capacity,
            },
            locations,
        }))
    })
}

// ---------------------------------------------------------------------- logs

/// Which part of the log to read.
#[derive(Debug, Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only records newer than this sequence number.
    #[serde(default)]
    pub after: Option<u64>,
    /// Lowest severity to include: `trace`, `debug`, `info`, `warn`, `error`.
    #[serde(default)]
    pub level: Option<String>,
}

/// Records returned when the caller does not say.
const DEFAULT_LOG_LIMIT: usize = 200;

/// The most one request can pull out of the buffer.
const MAX_LOG_LIMIT: usize = logs::CAPACITY;

pub(crate) async fn recent_logs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    QueryParams(query): QueryParams<LogQuery>,
) -> ApiResult<LogPage> {
    crate::routes::authorize(&state, &headers)?;

    let level = match query.level.as_deref() {
        None => None,
        Some(name) => Some(Severity::parse(name).ok_or_else(|| {
            Failure::new(StatusCode::BAD_REQUEST, format!("unknown level `{name}`"))
                .with_hint(Some("use trace, debug, info, warn or error".into()))
        })?),
    };

    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT);

    Ok(axum::Json(logs::buffer().page(limit, query.after, level)))
}

// ----------------------------------------------------------------- processes

/// A CtxC process this daemon can see but cannot stop.
///
/// The daemon reads the same records `ctxc stop` acts on, so a dashboard can
/// say what would still be running after the daemon goes away. It only reads:
/// ending a process is the CLI's job, and moving that here would mean a web
/// page could terminate processes on the machine serving it.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessView {
    pub pid: u32,
    /// The command it is running, as a user would type it: `mcp`, `start`.
    pub command: String,
    pub started_at: i64,
    /// True for the daemon answering this request, which stopping does cover.
    pub is_daemon: bool,
}

/// What is running for this data directory, and what to do about it.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessList {
    /// Everything recorded and still alive, the daemon included.
    pub processes: Vec<ProcessView>,
    /// How many would survive the daemon stopping.
    pub others: usize,
    /// The command that ends all of them.
    pub stop_command: String,
}

pub(crate) async fn processes(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<ProcessList> {
    crate::routes::authorize(&state, &headers)?;
    let data_dir = state.locations().data_dir.clone();

    // Every liveness check shells out to the operating system, so it runs off
    // the async runtime rather than blocking a request thread.
    let found = tokio::task::spawn_blocking(move || {
        let daemon = std::process::id();
        ctxc_core::processes::entries(&data_dir)
            .into_iter()
            .filter(ctxc_core::processes::Entry::is_alive)
            .map(|entry| ProcessView {
                is_daemon: entry.pid == daemon,
                pid: entry.pid,
                command: entry.command,
                started_at: entry.started_at,
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|err| {
        Failure::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not look up running processes: {err}"),
        )
    })?;

    Ok(axum::Json(ProcessList {
        others: found.iter().filter(|process| !process.is_daemon).count(),
        stop_command: "ctxc stop".to_string(),
        processes: found,
    }))
}
