/**
 * The dashboard's view of the daemon.
 *
 * Everything here goes through the same HTTP API the CLI uses, with the same
 * token and no privileges of its own. If a number can be seen here, it can be
 * seen from `ctxc metrics` too, and every button on a screen is one call to a
 * route `ctxc` can call as well — that is deliberate, and it is what keeps the
 * dashboard from growing logic that belongs in the core.
 */

/** How the daemon reports a failure, on every route. */
export interface ApiError {
  error: string;
  hint?: string;
}

export class RequestFailed extends Error {
  readonly status: number;
  readonly hint?: string;

  constructor(status: number, message: string, hint?: string) {
    super(message);
    this.name = "RequestFailed";
    this.status = status;
    this.hint = hint;
  }
}

/**
 * The access token for this session.
 *
 * `ctxc dashboard` puts it in the URL, because a browser cannot be told to send
 * a header. It is moved into session storage and stripped from the address bar
 * on arrival: leaving it there would put it in the history, in a bookmark, and
 * in whatever the user pastes when asking for help.
 */
function claimToken(): string {
  const url = new URL(window.location.href);
  const fromUrl = url.searchParams.get("token");

  if (fromUrl) {
    sessionStorage.setItem("ctxc-token", fromUrl);
    url.searchParams.delete("token");
    window.history.replaceState({}, "", url.pathname + url.hash);
    return fromUrl;
  }

  return sessionStorage.getItem("ctxc-token") ?? "";
}

export const token = claimToken();

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      ...init,
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
        ...init?.headers,
      },
    });
  } catch {
    // The daemon being gone is the common case here — it was stopped, or it
    // crashed — and "Failed to fetch" explains none of that.
    throw new RequestFailed(
      0,
      "the daemon is not answering",
      "check it is running with `ctxc daemon status`",
    );
  }

  if (!response.ok) {
    // Failures carry the daemon's own message and hint, so the dashboard shows
    // what the CLI would have shown rather than inventing its own wording.
    const body = await response.text();
    try {
      const parsed = JSON.parse(body) as ApiError;
      throw new RequestFailed(response.status, parsed.error, parsed.hint);
    } catch (cause) {
      if (cause instanceof RequestFailed) throw cause;
      // Not every failure is one of ours: a rejected request body is answered
      // by the web framework in plain text. Showing it beats "returned 422".
      throw new RequestFailed(
        response.status,
        body.trim() || `the daemon returned ${response.status}`,
      );
    }
  }

  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

// ---------------------------------------------------------------- types

export interface WatchReport {
  project: string;
  path: string;
  watching: boolean;
  degraded_reason?: string;
  pending_changes: number;
}

export interface DaemonStatus {
  version: string;
  pid: number;
  uptime_ms: number;
  started_at: number;
  projects: number;
  active_projects: number;
  indexed_files: number;
  schema_version: number;
  watching: number;
  degraded: number;
  watch: WatchReport[];
}

/** What CtxC recognised about a project when it was registered. */
export interface Detection {
  languages: string[];
  frameworks: string[];
  package_manager?: string;
  git: boolean;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  status: "active" | "paused";
  added_at: number;
  last_indexed_at?: number;
  detection: Detection;
  exists: boolean;
  indexed_files: number;
  symbols: number;
}

export interface IndexReport {
  root: string;
  scanned: number;
  indexed: number;
  unchanged: number;
  removed: number;
  ignored: number;
  too_large: number;
  unparsed: number;
  embedded: number;
  symbols: number;
  relationships: number;
  duration_ms: number;
}

export interface StageSavings {
  filtering: number;
  deduplication: number;
  compression: number;
  selection: number;
}

export interface CostEstimate {
  amount: number;
  currency: string;
  model: string;
  estimated: boolean;
}

export interface Summary {
  window: { from: number; to: number };
  project_id?: string;
  operations: number;
  input_tokens: number;
  output_tokens: number;
  tokens_saved: number;
  reduction_ratio: number;
  savings_by_stage: StageSavings;
  average_duration_ms?: number;
  slowest_duration_ms: number;
  errors: number;
  degradations: number;
  cache_hit_rate?: number;
  estimated_cost_saved?: CostEstimate;
  estimated: boolean;
}

export interface OperationSummary {
  operation: string;
  operations: number;
  input_tokens: number;
  output_tokens: number;
  tokens_saved: number;
  reduction_ratio: number;
  average_duration_ms?: number;
  errors: number;
  degradations: number;
}

export interface Breakdown {
  window: { from: number; to: number };
  project_id?: string;
  input_tokens: number;
  output_tokens: number;
  savings_by_stage: StageSavings;
  by_operation: OperationSummary[];
}

export interface TimeseriesPoint {
  bucket_start: number;
  operations: number;
  input_tokens: number;
  output_tokens: number;
  tokens_saved: number;
  reduction_ratio: number;
  savings_by_stage: StageSavings;
  errors: number;
  degradations: number;
}

export interface Timeseries {
  granularity: "hour" | "day";
  window: { from: number; to: number };
  project_id?: string;
  points: TimeseriesPoint[];
}

export interface MetricEvent {
  project_id?: string;
  operation: string;
  source: string;
  optimizer?: string;
  recorded_at: number;
  input_tokens: number;
  output_tokens: number;
  duration_ms: number;
  savings_by_stage: StageSavings;
  cache: { hits: number; lookups: number };
  outcome: "success" | "degraded" | "failed";
  detail?: string;
  estimated: boolean;
}

// ------------------------------------------------------------ configuration

export interface RankingConfig {
  keyword: number;
  semantic: number;
  symbol: number;
  graph: number;
  recency: number;
  hop_decay: number;
  expansion_depth: number;
  recency_half_life_days: number;
}

export interface Config {
  core: { mode: string };
  optimization: { enabled: boolean; target_reduction: number };
  retrieval: { enabled: boolean };
  ranking: RankingConfig;
  graph: { enabled: boolean };
  storage: { path: string };
  budget: { default: number };
  daemon: { enabled: boolean; auto_start: boolean; bind: string; port: number };
  watch: { enabled: boolean; debounce_ms: number; poll_interval_ms: number };
  semantic: {
    enabled: boolean;
    provider: string;
    dimensions: number;
    redundancy_threshold: number;
    diversity: number;
  };
  metrics: {
    enabled: boolean;
    raw_retention_days: number;
    hourly_retention_days: number;
    cost_model: string;
    cost_per_million_input_tokens: number;
    cost_currency: string;
  };
  dashboard: { enabled: boolean; port: number };
  telemetry: { enabled: boolean };
}

/** A section of a configuration layer: every key optional. */
export type PartialConfig = {
  [Section in keyof Config]?: Partial<Config[Section]>;
};

export interface Locations {
  config_file: string;
  config_dir: string;
  data_dir: string;
  cache_dir: string;
  database: string;
}

export interface LayerInfo {
  kind: "defaults" | "file" | "environment" | "overrides";
  path?: string;
  applied: boolean;
}

export interface ConfigView {
  running: Config;
  effective: Config;
  toml: string;
  file: { path: string; exists: boolean; keys: string[]; toml: string };
  environment_keys: string[];
  editable_keys: string[];
  layers: LayerInfo[];
  locations: Locations;
  restart_required: boolean;
}

export interface ConfigEdited extends ConfigView {
  changed: string[];
  shadowed: string[];
  created: boolean;
}

// ---------------------------------------------------------------- commands

export interface ArgumentInfo {
  name: string;
  help?: string;
  required: boolean;
  repeated: boolean;
}

export interface OptionInfo {
  name: string;
  short?: string;
  help?: string;
  value_name?: string;
  default?: string;
  values?: string[];
  required: boolean;
}

export interface DashboardEquivalent {
  route: string;
  label: string;
}

export interface CommandInfo {
  path: string;
  name: string;
  summary?: string;
  description?: string;
  usage: string;
  arguments?: ArgumentInfo[];
  options?: OptionInfo[];
  examples?: string[];
  subcommands?: CommandInfo[];
  dashboard?: DashboardEquivalent;
}

export interface Catalog {
  name: string;
  version: string;
  about?: string;
  global_options: OptionInfo[];
  commands: CommandInfo[];
}

// ------------------------------------------------------------ context, index

export interface Signals {
  keyword: number;
  semantic: number;
  symbol: number;
  graph: number;
  recency: number;
  hops: number;
}

export type Reason =
  | { kind: "content" }
  | { kind: "symbol" }
  | { kind: "content_and_symbol" }
  | { kind: "related"; to: string; hops: number };

export interface RetrievedFile {
  path: string;
  language?: string;
  score: number;
  signals: Signals;
  reason: Reason;
  matched_symbols: string[];
  line?: number;
  snippet?: string;
}

export interface Retrieval {
  query: string;
  root: string;
  considered: number;
  files: RetrievedFile[];
}

export interface StoredContext {
  id: string;
  reference: string;
  source: string;
  content_type: string;
  bytes: number;
  content: string;
}

export interface IndexedFileView {
  path: string;
  language?: string;
  content: string;
  dependencies: string[];
  dependents: string[];
}

export interface GraphView {
  files: number;
  edges: number;
  most_depended_on: { path: string; dependents: number }[];
  file?: { path: string; dependencies: string[]; dependents: string[] };
}

// --------------------------------------------------------- system, logging

export interface Diagnostics {
  version: string;
  os: string;
  arch: string;
  pid: number;
  uptime_ms: number;
  started_at: number;
  locations: Locations;
  config_file_exists: boolean;
  database: { path: string; schema_version: number; size_bytes?: number };
  indexed_roots: number;
  contexts: number;
  routes: string[];
  logs: { held: number; dropped: number; capacity: number };
}

/**
 * A CtxC process the daemon can see but cannot stop.
 *
 * The daemon reads the records `ctxc stop` acts on, so this page can say what
 * would survive the daemon going away. Ending one is the CLI's job.
 */
export interface ProcessView {
  pid: number;
  command: string;
  started_at: number;
  is_daemon: boolean;
}

export interface ProcessList {
  processes: ProcessView[];
  /** How many would still be running once the daemon stops. */
  others: number;
  stop_command: string;
}

export type LogLevel = "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR";

export interface LogRecord {
  seq: number;
  at: number;
  level: LogLevel;
  target: string;
  message: string;
  fields?: Record<string, string>;
}

export interface LogPage {
  records: LogRecord[];
  dropped: number;
  capacity: number;
}

// ---------------------------------------------------------------- calls

const scoped = (project?: string) =>
  project ? `&project=${encodeURIComponent(project)}` : "";

const id = (value: string) => encodeURIComponent(value);

export const api = {
  status: () => request<DaemonStatus>("/v1/status"),

  projects: () => request<Project[]>("/v1/projects"),

  project: (project: string) => request<Project>(`/v1/projects/${id(project)}`),

  addProject: (path: string) =>
    request<Project>("/v1/projects", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),

  removeProject: (project: string) =>
    request<Project>(`/v1/projects/${id(project)}`, { method: "DELETE" }),

  pauseProject: (project: string) =>
    request<Project>(`/v1/projects/${id(project)}/pause`, { method: "POST" }),

  resumeProject: (project: string) =>
    request<Project>(`/v1/projects/${id(project)}/resume`, { method: "POST" }),

  reindexProject: (project: string) =>
    request<IndexReport>(`/v1/projects/${id(project)}/reindex`, {
      method: "POST",
    }),

  projectFile: (project: string, path: string) =>
    request<IndexedFileView>(
      `/v1/projects/${id(project)}/file?path=${encodeURIComponent(path)}`,
    ),

  projectGraph: (project: string, limit = 10, file?: string) =>
    request<GraphView>(
      `/v1/projects/${id(project)}/graph?limit=${limit}` +
        (file ? `&file=${encodeURIComponent(file)}` : ""),
    ),

  search: (project: string, query: string, limit = 20) =>
    request<Retrieval>("/v1/context/search", {
      method: "POST",
      body: JSON.stringify({ project, query, limit }),
    }),

  context: (reference: string) =>
    request<StoredContext>(`/v1/contexts/${id(reference)}`),

  summary: (days: number, project?: string) =>
    request<Summary>(`/v1/metrics/summary?days=${days}${scoped(project)}`),

  breakdown: (days: number, project?: string) =>
    request<Breakdown>(`/v1/metrics/breakdown?days=${days}${scoped(project)}`),

  timeseries: (granularity: "hour" | "day", days: number, project?: string) =>
    request<Timeseries>(
      `/v1/metrics/timeseries?granularity=${granularity}&days=${days}${scoped(project)}`,
    ),

  activity: (limit: number, project?: string) =>
    request<MetricEvent[]>(`/v1/activity?limit=${limit}${scoped(project)}`),

  config: () => request<ConfigView>("/v1/config"),

  editConfig: (set: PartialConfig, reset: string[] = []) =>
    request<ConfigEdited>("/v1/config", {
      method: "PATCH",
      body: JSON.stringify({ set, reset }),
    }),

  commands: () => request<Catalog>("/v1/commands"),

  diagnostics: () => request<Diagnostics>("/v1/diagnostics"),

  logs: (limit = 200, level?: string, after?: number) =>
    request<LogPage>(
      `/v1/logs?limit=${limit}` +
        (level ? `&level=${encodeURIComponent(level)}` : "") +
        (after ? `&after=${after}` : ""),
    ),

  processes: () => request<ProcessList>("/v1/processes"),

  shutdown: () =>
    request<{
      stopping: boolean;
      pid: number;
      still_running: number;
      stop_command: string;
    }>("/v1/shutdown", {
      method: "POST",
    }),
};
