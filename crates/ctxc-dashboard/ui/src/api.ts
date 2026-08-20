/**
 * The dashboard's view of the daemon.
 *
 * Everything here goes through the same HTTP API the CLI uses, with the same
 * token and no privileges of its own. If a number can be seen here, it can be
 * seen from `ctxc metrics` too — that is deliberate, and it is what keeps the
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
  const response = await fetch(path, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      ...init?.headers,
    },
  });

  if (!response.ok) {
    // Failures carry the daemon's own message and hint, so the dashboard shows
    // what the CLI would have shown rather than inventing its own wording.
    let body: ApiError = { error: `the daemon returned ${response.status}` };
    try {
      body = (await response.json()) as ApiError;
    } catch {
      // A response that is not JSON is still a failure worth reporting.
    }
    throw new RequestFailed(response.status, body.error, body.hint);
  }

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

export interface Project {
  id: string;
  name: string;
  path: string;
  status: "active" | "paused";
  added_at: number;
  last_indexed_at?: number;
  languages: string[];
  frameworks: string[];
  package_manager?: string;
  has_git: boolean;
  exists: boolean;
  indexed_files: number;
  symbols: number;
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

// ---------------------------------------------------------------- calls

const scoped = (project?: string) =>
  project ? `&project=${encodeURIComponent(project)}` : "";

export const api = {
  status: () => request<DaemonStatus>("/v1/status"),

  projects: () => request<Project[]>("/v1/projects"),

  addProject: (path: string) =>
    request<Project>("/v1/projects", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),

  removeProject: (id: string) =>
    request<Project>(`/v1/projects/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),

  pauseProject: (id: string) =>
    request<Project>(`/v1/projects/${encodeURIComponent(id)}/pause`, {
      method: "POST",
    }),

  resumeProject: (id: string) =>
    request<Project>(`/v1/projects/${encodeURIComponent(id)}/resume`, {
      method: "POST",
    }),

  reindexProject: (id: string) =>
    request<unknown>(`/v1/projects/${encodeURIComponent(id)}/reindex`, {
      method: "POST",
    }),

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
};
