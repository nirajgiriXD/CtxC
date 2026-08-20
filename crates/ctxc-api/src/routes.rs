//! The local HTTP API.
//!
//! Versioned under `/v1`, bound to loopback, and behind a bearer token that the
//! daemon generates at startup. The dashboard will be a client of exactly this
//! API with no special privileges — which is the point of putting it behind
//! HTTP rather than reaching into the engine.
//!
//! Handlers are thin: they translate a request into a call on the registry, the
//! index or the engine, and translate the answer back. Nothing here decides
//! policy.

use std::path::{Path, PathBuf};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as PathParam, Query as QueryParams, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use ctxc_core::{ContextSource, TokenBudget};
use ctxc_engine::index::{IndexOptions, IndexReport, Indexer};
use ctxc_engine::Engine;
use ctxc_metrics::report::{Breakdown, Summary, Timeseries};
use ctxc_metrics::{Granularity, MetricEvent, Metrics, Operation, Window};
use ctxc_project::{Project, Registry};
use ctxc_retrieval::{Query, Retrieval, RetrievalOptions, Retriever};
use ctxc_store::{IndexStore, SqliteIndexStore, SqliteMetricsStore, SqliteProjectStore};

use crate::state::ApiState;

/// Build the router.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/projects", get(list_projects).post(add_project))
        .route("/v1/projects/{id}", get(project).delete(remove_project))
        .route("/v1/projects/{id}/pause", post(pause_project))
        .route("/v1/projects/{id}/resume", post(resume_project))
        .route("/v1/projects/{id}/reindex", post(reindex_project))
        .route("/v1/context/search", post(search))
        .route("/v1/context/optimize", post(optimize))
        .route("/v1/metrics/summary", get(metrics_summary))
        .route("/v1/metrics/projects/{id}", get(project_metrics))
        .route("/v1/metrics/timeseries", get(metrics_timeseries))
        .route("/v1/metrics/breakdown", get(metrics_breakdown))
        .route("/v1/activity", get(activity))
        .route("/v1/events", get(events))
        .route("/v1/shutdown", post(shutdown))
        // The dashboard is served from the root, under everything versioned.
        // It is a client of the API above it and gets no privileges from
        // sharing a port — only the convenience of one.
        .route("/", get(dashboard_index))
        .route("/{*path}", get(dashboard_asset))
        .with_state(state)
}

/// What every failure looks like on the wire.
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The same shape, read back by a client.
///
/// Separate from [`ApiError`] only because the server never needs to
/// deserialize its own errors and the client never needs to build one.
#[derive(Debug, Deserialize)]
pub struct ApiErrorBody {
    pub error: String,
    #[serde(default)]
    pub hint: Option<String>,
}

/// A failure, with the status it should be reported as.
pub struct Failure(StatusCode, ApiError);

impl Failure {
    fn new(status: StatusCode, error: impl Into<String>) -> Failure {
        Failure(
            status,
            ApiError {
                error: error.into(),
                hint: None,
            },
        )
    }

    fn with_hint(mut self, hint: Option<String>) -> Failure {
        self.1.hint = hint;
        self
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<ctxc_project::ProjectError> for Failure {
    fn from(error: ctxc_project::ProjectError) -> Self {
        let status = match error {
            ctxc_project::ProjectError::NotRegistered { .. } => StatusCode::NOT_FOUND,
            ctxc_project::ProjectError::NoSuchDirectory { .. }
            | ctxc_project::ProjectError::NotADirectory { .. }
            | ctxc_project::ProjectError::Ambiguous { .. }
            | ctxc_project::ProjectError::Config { .. } => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let hint = error.hint();
        Failure::new(status, error.to_string()).with_hint(hint)
    }
}

impl From<ctxc_store::StoreError> for Failure {
    fn from(error: ctxc_store::StoreError) -> Self {
        let hint = error.hint();
        Failure::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).with_hint(hint)
    }
}

impl From<ctxc_engine::EngineError> for Failure {
    fn from(error: ctxc_engine::EngineError) -> Self {
        let hint = error.hint();
        Failure::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).with_hint(hint)
    }
}

impl From<ctxc_retrieval::RetrievalError> for Failure {
    fn from(error: ctxc_retrieval::RetrievalError) -> Self {
        let hint = error.hint();
        Failure::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).with_hint(hint)
    }
}

impl From<ctxc_metrics::MetricsError> for Failure {
    fn from(error: ctxc_metrics::MetricsError) -> Self {
        let status = match error {
            ctxc_metrics::MetricsError::BadWindow { .. } => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let hint = error.hint();
        Failure::new(status, error.to_string()).with_hint(hint)
    }
}

type ApiResult<T> = Result<Json<T>, Failure>;

/// Check the bearer token.
///
/// `/v1/health` is the one route that does not require it: something has to be
/// answerable before a client knows whether a daemon is even there.
fn authorize(state: &ApiState, headers: &HeaderMap) -> Result<(), Failure> {
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();

    if state.token().matches(presented) {
        Ok(())
    } else {
        Err(
            Failure::new(StatusCode::UNAUTHORIZED, "invalid or missing token")
                .with_hint(Some("the token is in the daemon's lockfile".into())),
        )
    }
}

/// Liveness, and the identity of what is answering.
#[derive(Debug, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
    pub pid: u32,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        pid: std::process::id(),
    })
}

/// What the daemon is currently doing.
#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    pub pid: u32,
    pub uptime_ms: u64,
    pub started_at: i64,
    pub projects: usize,
    pub active_projects: usize,
    pub indexed_files: u64,
    pub schema_version: u32,
    /// Projects being watched, and any that fell back to scanning.
    pub watching: usize,
    pub degraded: usize,
    pub watch: Vec<crate::state::WatchReport>,
}

async fn status(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<DaemonStatus> {
    authorize(&state, &headers)?;
    let watch = state.watching();

    state.with_database(|database| {
        let projects = Registry::new(&SqliteProjectStore::new(database)).list()?;
        let store = SqliteIndexStore::new(database);

        let indexed_files = projects
            .iter()
            .filter_map(|project| store.counts(&project.path).ok())
            .map(|counts| counts.files)
            .sum();

        Ok(Json(DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            uptime_ms: state.uptime_ms(),
            started_at: state.started_at().as_millis(),
            active_projects: projects.iter().filter(|p| p.status.is_active()).count(),
            projects: projects.len(),
            indexed_files,
            schema_version: database.schema_version()?,
            watching: watch.iter().filter(|report| report.watching).count(),
            degraded: watch.iter().filter(|report| !report.watching).count(),
            watch,
        }))
    })
}

/// A project, with what the index knows about it.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub exists: bool,
    pub indexed_files: u64,
    pub symbols: u64,
}

/// Announce that a project changed, and build its view.
///
/// The dashboard manages projects through this API, so a change made in one
/// browser tab has to reach the others without a refresh. `status` is spelled
/// out rather than taken from the project because "removed" is not a status a
/// project can still be holding.
fn changed(
    project: Project,
    status: &str,
    state: &ApiState,
    store: &dyn IndexStore,
) -> ProjectView {
    state.publish(project_event(&project, status));
    view(project, store)
}

/// The event describing one project's new state.
fn project_event(project: &Project, status: &str) -> crate::events::StreamEvent {
    crate::events::StreamEvent::Project {
        id: project.id.as_str().to_owned(),
        name: project.name.clone(),
        status: status.to_owned(),
    }
}

fn view(project: Project, store: &dyn IndexStore) -> ProjectView {
    let counts = store.counts(&project.path).unwrap_or_default();
    ProjectView {
        exists: project.exists(),
        indexed_files: counts.files,
        symbols: counts.symbols,
        project,
    }
}

async fn list_projects(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<ProjectView>> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let projects = Registry::new(&SqliteProjectStore::new(database)).list()?;
        let store = SqliteIndexStore::new(database);
        Ok(Json(
            projects
                .into_iter()
                .map(|project| view(project, &store))
                .collect(),
        ))
    })
}

/// Register a project.
#[derive(Debug, Serialize, Deserialize)]
pub struct AddProject {
    pub path: String,
}

async fn add_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<AddProject>,
) -> ApiResult<ProjectView> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let registration =
            Registry::new(&SqliteProjectStore::new(database)).add(Path::new(&request.path))?;
        let store = SqliteIndexStore::new(database);
        Ok(Json(changed(
            registration.project,
            "active",
            &state,
            &store,
        )))
    })
}

async fn project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
) -> ApiResult<ProjectView> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).resolve(&id)?;
        let store = SqliteIndexStore::new(database);
        Ok(Json(view(project, &store)))
    })
}

async fn remove_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
) -> ApiResult<ProjectView> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).remove(&id)?;
        let store = SqliteIndexStore::new(database);
        Ok(Json(changed(project, "removed", &state, &store)))
    })
}

async fn pause_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
) -> ApiResult<ProjectView> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).pause(&id)?;
        let store = SqliteIndexStore::new(database);
        Ok(Json(changed(project, "paused", &state, &store)))
    })
}

async fn resume_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
) -> ApiResult<ProjectView> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).resume(&id)?;
        let store = SqliteIndexStore::new(database);
        Ok(Json(changed(project, "active", &state, &store)))
    })
}

async fn reindex_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
) -> ApiResult<IndexReport> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let projects = SqliteProjectStore::new(database);
        let registry = Registry::new(&projects);
        let project = registry.resolve(&id)?;
        let store = SqliteIndexStore::new(database);

        let report = database.transaction(|| {
            Indexer::new(&store).index(&PathBuf::from(&project.path), &IndexOptions::default())
        })?;
        registry.record_indexed(&project.id, ctxc_core::Timestamp::now())?;

        // Files that had not changed are the index cache doing its job.
        let looked_at = report.indexed.saturating_add(report.unchanged);
        state.record(
            MetricEvent::new(Operation::Index, &project.path)
                .for_project(project.id.as_str())
                .took(std::time::Duration::from_millis(report.duration_ms))
                .with_cache(
                    report.unchanged.min(u32::MAX as u64) as u32,
                    looked_at.min(u32::MAX as u64) as u32,
                ),
        );

        Ok(Json(report))
    })
}

/// Ask for the context relevant to a question.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    /// Project id, path or name.
    pub project: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

async fn search(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<SearchRequest>,
) -> ApiResult<Retrieval> {
    authorize(&state, &headers)?;

    state.with_database(|database| {
        let project =
            Registry::new(&SqliteProjectStore::new(database)).resolve(&request.project)?;
        let store = SqliteIndexStore::new(database);

        let mut options = RetrievalOptions::from_config(state.config());
        if let Some(limit) = request.limit {
            options.limit = limit;
        }

        let started = std::time::Instant::now();
        let retrieval = Retriever::new(&store, options).retrieve(
            &project.path,
            &Query::parse(&request.query),
            ctxc_core::Timestamp::now().as_millis(),
        )?;

        state.record(
            MetricEvent::new(Operation::Search, &request.query)
                .for_project(project.id.as_str())
                .took(started.elapsed()),
        );

        Ok(Json(retrieval))
    })
}

/// Optimize content the caller already has.
#[derive(Debug, Serialize, Deserialize)]
pub struct OptimizeRequest {
    pub content: String,
    #[serde(default)]
    pub budget: Option<u32>,
    /// The command that produced the content, when there was one.
    #[serde(default)]
    pub from: Option<String>,
}

/// The optimized content and what it cost.
#[derive(Debug, Serialize, Deserialize)]
pub struct OptimizeResponse {
    pub content: String,
    pub reference: String,
    pub result: ctxc_core::OptimizationResult,
}

async fn optimize(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<OptimizeRequest>,
) -> ApiResult<OptimizeResponse> {
    authorize(&state, &headers)?;

    let source = match request.from {
        Some(command) => ContextSource::Command { command },
        None => ContextSource::Api {
            endpoint: "/v1/context/optimize".into(),
        },
    };
    let context = ctxc_context::ingest::from_text(source, &request.content, None);

    let engine = Engine::from_config(state.config());
    let started = std::time::Instant::now();
    let optimized = engine.optimize(&context, request.budget.map(TokenBudget::new))?;

    state.record(
        MetricEvent::from_result(
            Operation::Optimize,
            ctxc_context::ingest::label(&context.metadata.source),
            &optimized.result,
        )
        .took(started.elapsed()),
    );

    Ok(Json(OptimizeResponse {
        reference: optimized.reference(),
        content: optimized.content,
        result: optimized.result,
    }))
}

/// Common query parameters for the metrics endpoints.
#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    /// How far back to look. Defaults to 30 days.
    #[serde(default)]
    pub days: Option<u32>,
    /// Restrict to one project, by id, path or name.
    #[serde(default)]
    pub project: Option<String>,
    /// `hour` or `day`. Timeseries only.
    #[serde(default)]
    pub granularity: Option<String>,
    /// Rows returned. Activity only.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Days a metrics query covers when it does not say.
const DEFAULT_METRICS_DAYS: u32 = 30;

/// Events returned by `/v1/activity` when the caller does not say.
const DEFAULT_ACTIVITY_LIMIT: usize = 50;

/// The most events one request can ask for, so a client cannot pull the whole
/// raw table into memory through a query string.
const MAX_ACTIVITY_LIMIT: usize = 1_000;

impl MetricsQuery {
    fn window(&self) -> Window {
        Window::last_days(self.days.unwrap_or(DEFAULT_METRICS_DAYS))
    }

    fn granularity(&self) -> Result<Granularity, Failure> {
        match self.granularity.as_deref() {
            None => Ok(Granularity::Hour),
            Some(value) => value
                .parse()
                .map_err(|err: ctxc_metrics::event::UnknownName| {
                    Failure::new(StatusCode::BAD_REQUEST, err.to_string())
                        .with_hint(Some("use `hour` or `day`".into()))
                }),
        }
    }

    fn limit(&self) -> usize {
        self.limit
            .unwrap_or(DEFAULT_ACTIVITY_LIMIT)
            .clamp(1, MAX_ACTIVITY_LIMIT)
    }
}

/// Resolve the `project` parameter to an id, or `None` for every project.
///
/// Resolution goes through the registry so that a path or a name works here
/// exactly as it does on the command line.
fn scope(
    database: &ctxc_store::Database,
    project: Option<&String>,
) -> Result<Option<String>, Failure> {
    match project {
        None => Ok(None),
        Some(reference) => {
            let store = SqliteProjectStore::new(database);
            let project = Registry::new(&store).resolve(reference)?;
            Ok(Some(project.id.as_str().to_owned()))
        }
    }
}

/// Make everything recorded so far visible to a report.
///
/// Buffered events are written and aggregates are brought up to date, because a
/// dashboard asking "what has CtxC saved?" must not be told a number that stops
/// at the daemon's last five-minute tick. Pruning is deliberately not part of
/// this: retention is the daemon's decision, not a side effect of a GET.
fn freshen(state: &ApiState) -> Result<(), Failure> {
    state.flush_metrics();
    state.with_database(|database| {
        let store = SqliteMetricsStore::new(database);
        Metrics::new(&store, state.config()).refresh()?;
        Ok(())
    })
}

async fn metrics_summary(
    State(state): State<ApiState>,
    headers: HeaderMap,
    QueryParams(query): QueryParams<MetricsQuery>,
) -> ApiResult<Summary> {
    authorize(&state, &headers)?;
    freshen(&state)?;

    state.with_database(|database| {
        let project = scope(database, query.project.as_ref())?;
        let store = SqliteMetricsStore::new(database);
        let metrics = Metrics::new(&store, state.config());
        Ok(Json(metrics.summary(query.window(), project.as_deref())?))
    })
}

async fn project_metrics(
    State(state): State<ApiState>,
    headers: HeaderMap,
    PathParam(id): PathParam<String>,
    QueryParams(query): QueryParams<MetricsQuery>,
) -> ApiResult<Summary> {
    authorize(&state, &headers)?;
    freshen(&state)?;

    state.with_database(|database| {
        let project = Registry::new(&SqliteProjectStore::new(database)).resolve(&id)?;
        let store = SqliteMetricsStore::new(database);
        let metrics = Metrics::new(&store, state.config());
        Ok(Json(
            metrics.summary(query.window(), Some(project.id.as_str()))?,
        ))
    })
}

async fn metrics_timeseries(
    State(state): State<ApiState>,
    headers: HeaderMap,
    QueryParams(query): QueryParams<MetricsQuery>,
) -> ApiResult<Timeseries> {
    authorize(&state, &headers)?;
    freshen(&state)?;
    let granularity = query.granularity()?;

    state.with_database(|database| {
        let project = scope(database, query.project.as_ref())?;
        let store = SqliteMetricsStore::new(database);
        let metrics = Metrics::new(&store, state.config());
        Ok(Json(metrics.timeseries(
            granularity,
            query.window(),
            project.as_deref(),
        )?))
    })
}

async fn metrics_breakdown(
    State(state): State<ApiState>,
    headers: HeaderMap,
    QueryParams(query): QueryParams<MetricsQuery>,
) -> ApiResult<Breakdown> {
    authorize(&state, &headers)?;
    freshen(&state)?;

    state.with_database(|database| {
        let project = scope(database, query.project.as_ref())?;
        let store = SqliteMetricsStore::new(database);
        let metrics = Metrics::new(&store, state.config());
        Ok(Json(metrics.breakdown(query.window(), project.as_deref())?))
    })
}

/// Recent operations, newest first.
async fn activity(
    State(state): State<ApiState>,
    headers: HeaderMap,
    QueryParams(query): QueryParams<MetricsQuery>,
) -> ApiResult<Vec<MetricEvent>> {
    authorize(&state, &headers)?;

    // Raw events, so a flush is enough — there is nothing to aggregate. An
    // activity feed that omits what just happened is the one thing it must not
    // do.
    state.flush_metrics();

    state.with_database(|database| {
        let project = scope(database, query.project.as_ref())?;
        let store = SqliteMetricsStore::new(database);
        let metrics = Metrics::new(&store, state.config());
        Ok(Json(metrics.activity(project.as_deref(), query.limit())?))
    })
}

/// Serve the dashboard's entry point.
async fn dashboard_index() -> Response {
    serve_asset(ctxc_dashboard::entry_point())
}

/// Serve one dashboard file.
///
/// Unknown paths fall back to the entry point, because the dashboard routes in
/// the browser and a deep link is a route rather than a missing file. Requests
/// that are plainly for an asset do not: answering a missing script with HTML
/// turns a clear 404 into a baffling syntax error in the console.
async fn dashboard_asset(PathParam(path): PathParam<String>) -> Response {
    match ctxc_dashboard::asset(&path) {
        Some(asset) => serve_asset(Some(asset)),
        None if looks_like_a_file(&path) => (StatusCode::NOT_FOUND, "not found").into_response(),
        None => serve_asset(ctxc_dashboard::entry_point()),
    }
}

/// Whether a path is asking for a file rather than a page.
///
/// Everything Vite emits lives under `assets/`, and anything with an extension
/// is a file by intent. A client-side route has neither.
fn looks_like_a_file(path: &str) -> bool {
    path.starts_with("assets/")
        || path
            .rsplit('/')
            .next()
            .map(|name| name.contains('.'))
            .unwrap_or(false)
}

/// Turn an embedded asset into a response, or explain that none was bundled.
fn serve_asset(asset: Option<&'static ctxc_dashboard::Asset>) -> Response {
    let Some(asset) = asset else {
        // A source build with no `npm run build`. Saying so beats a blank page
        // that looks like a bug in the daemon.
        return (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            "This build of CtxC does not include the dashboard.\n\n\
             Build it with `npm --prefix crates/ctxc-dashboard/ui ci` and \
             `npm --prefix crates/ctxc-dashboard/ui run build`, then rebuild ctxc.\n\n\
             Everything else, including the API and the CLI, works without it.\n",
        )
            .into_response();
    };

    (
        [
            (axum::http::header::CONTENT_TYPE, asset.content_type),
            (axum::http::header::CACHE_CONTROL, asset.cache_control()),
            // The dashboard renders repository content, which is arbitrary
            // text from someone's files. Nothing here should ever be sniffed
            // into a different type than it was declared as.
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        asset.bytes,
    )
        .into_response()
}

/// How a WebSocket client presents its token.
///
/// Browsers cannot set headers on a WebSocket handshake, so the token comes in
/// the query string instead. It never leaves the loopback interface, and it is
/// the same token every other route requires.
#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub token: Option<String>,
}

/// Subscribe to what the daemon is doing.
///
/// Authorisation happens before the upgrade, so a client with no token is
/// refused with a normal HTTP 401 rather than an opaque closed socket.
async fn events(
    State(state): State<ApiState>,
    headers: HeaderMap,
    QueryParams(query): QueryParams<StreamQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let presented = query.token.as_deref().unwrap_or_default();
    if !state.token().matches(presented) && authorize(&state, &headers).is_err() {
        return Failure::new(StatusCode::UNAUTHORIZED, "invalid or missing token")
            .with_hint(Some(
                "pass ?token=<token> from the daemon's lockfile".into(),
            ))
            .into_response();
    }

    upgrade.on_upgrade(move |socket| stream_events(socket, state))
}

/// Forward events to one client until it goes away.
async fn stream_events(mut socket: WebSocket, state: ApiState) {
    let mut subscriber = state.events().subscribe();
    tracing::debug!(
        listeners = state.events().listeners(),
        "event stream opened"
    );

    loop {
        tokio::select! {
            // A client that has closed, or that is sending us something, ends
            // the stream. Nothing it can say is part of the protocol.
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            },

            published = subscriber.next() => {
                let Some(envelope) = published else {
                    break;
                };
                let Ok(text) = serde_json::to_string(&envelope) else {
                    // An event that cannot be serialized is a bug in the
                    // event type, not a reason to drop the connection.
                    tracing::warn!("could not serialize an event for the stream");
                    continue;
                };
                if socket.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        }
    }

    tracing::debug!("event stream closed");
}

/// Stop the daemon.
#[derive(Debug, Serialize, Deserialize)]
pub struct ShutdownResponse {
    pub stopping: bool,
    pub pid: u32,
}

async fn shutdown(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<ShutdownResponse> {
    authorize(&state, &headers)?;

    // Windows has no portable signal for "stop gracefully", so the API is how a
    // client asks — which also means `ctxc stop` behaves the same everywhere.
    state.request_shutdown();
    Ok(Json(ShutdownResponse {
        stopping: true,
        pid: std::process::id(),
    }))
}

/// The routes this build answers, for diagnostics and documentation.
pub fn routes() -> Vec<&'static str> {
    vec![
        "GET /v1/health",
        "GET /v1/status",
        "GET /v1/projects",
        "POST /v1/projects",
        "GET /v1/projects/{id}",
        "DELETE /v1/projects/{id}",
        "POST /v1/projects/{id}/pause",
        "POST /v1/projects/{id}/resume",
        "POST /v1/projects/{id}/reindex",
        "POST /v1/context/search",
        "POST /v1/context/optimize",
        "GET /v1/metrics/summary",
        "GET /v1/metrics/projects/{id}",
        "GET /v1/metrics/timeseries",
        "GET /v1/metrics/breakdown",
        "GET /v1/activity",
        "WS /v1/events",
        "POST /v1/shutdown",
        "GET / (dashboard)",
    ]
}
