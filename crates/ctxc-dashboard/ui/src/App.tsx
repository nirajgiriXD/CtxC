/**
 * The dashboard.
 *
 * A client of the daemon's HTTP API with no privileges of its own: everything
 * on screen came from a route the CLI can call too. The WebSocket is only a
 * notification that something changed — each panel re-reads what it needs, so
 * a dropped frame costs a delay rather than a wrong number.
 */

import { useState } from "react";
import { Activity as ActivityIcon, FolderGit2, Gauge } from "lucide-react";

import { api, token } from "./api";
import { useEventStream } from "./events";
import { ago, duration } from "./format";
import { useApi } from "./useApi";
import { Activity } from "./components/Activity";
import { Overview } from "./components/Overview";
import { Projects } from "./components/Projects";
import { Badge, Card, Failure, Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui";

const WINDOWS = [
  { days: 1, label: "24h" },
  { days: 7, label: "7d" },
  { days: 30, label: "30d" },
  { days: 90, label: "90d" },
];

export function App() {
  const stream = useEventStream();
  const [days, setDays] = useState(7);
  const [project, setProject] = useState<string | undefined>();

  const status = useApi(() => api.status(), [stream.revision]);

  // Without a token nothing else can work, and every panel would show the same
  // 401. Say it once, at the top, with the command that fixes it.
  if (!token) {
    return (
      <Shell>
        <Failure
          message="No access token."
          hint="open the dashboard with `ctxc dashboard`, which puts the token in the URL"
        />
      </Shell>
    );
  }

  const projectName = status.data?.watch.find(
    (report) => report.project === project,
  )?.project;

  return (
    <Shell>
      <header className="mb-6 flex flex-wrap items-center gap-4">
        <div className="flex items-center gap-3">
          <h1 className="text-lg font-semibold tracking-tight">CtxC</h1>
          {status.data ? (
            <span className="text-muted-foreground text-xs">
              v{status.data.version} · pid {status.data.pid} · up{" "}
              {duration(status.data.uptime_ms)}
            </span>
          ) : null}
        </div>

        <div className="flex flex-1 items-center justify-end gap-3">
          {stream.state === "live" ? (
            <Badge variant="success">Live</Badge>
          ) : stream.state === "connecting" ? (
            <Badge>Connecting…</Badge>
          ) : (
            <Badge variant="warning">Reconnecting…</Badge>
          )}

          <div className="bg-muted/50 flex gap-1 rounded-lg p-1">
            {WINDOWS.map((option) => (
              <button
                key={option.days}
                onClick={() => setDays(option.days)}
                className={[
                  "rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
                  days === option.days
                    ? "bg-card text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                ].join(" ")}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>
      </header>

      {status.error ? (
        <div className="mb-4">
          <Failure
            message={status.error.message}
            hint={"hint" in status.error ? status.error.hint : undefined}
            onRetry={status.reload}
          />
        </div>
      ) : null}

      {status.data ? (
        <div className="mb-6 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Fact label="Projects" value={`${status.data.active_projects} of ${status.data.projects} active`} />
          <Fact label="Watching" value={`${status.data.watching} watched, ${status.data.degraded} polling`} />
          <Fact label="Files indexed" value={status.data.indexed_files.toLocaleString("en-US")} />
          <Fact label="Started" value={ago(status.data.started_at)} />
        </div>
      ) : null}

      {project ? (
        <div className="mb-4 flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">Showing only</span>
          <Badge>{projectName ?? project}</Badge>
          <button
            onClick={() => setProject(undefined)}
            className="text-muted-foreground hover:text-foreground text-xs underline"
          >
            show every project
          </button>
        </div>
      ) : null}

      <Tabs defaultValue="overview">
        <TabsList className="mb-4">
          <TabsTrigger value="overview">
            <Gauge className="size-4" /> Overview
          </TabsTrigger>
          <TabsTrigger value="projects">
            <FolderGit2 className="size-4" /> Projects
          </TabsTrigger>
          <TabsTrigger value="activity">
            <ActivityIcon className="size-4" /> Activity
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview">
          <Overview days={days} project={project} revision={stream.revision} />
        </TabsContent>

        <TabsContent value="projects">
          <Projects
            watching={status.data?.watch ?? []}
            revision={stream.revision}
            selected={project}
            onSelect={setProject}
          />
        </TabsContent>

        <TabsContent value="activity">
          <Activity
            live={stream.recent}
            project={project}
            connection={stream.state}
          />
        </TabsContent>
      </Tabs>
    </Shell>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="mx-auto min-h-screen w-full max-w-7xl px-4 py-6 sm:px-6">
      {children}
    </div>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <Card className="px-4 py-3">
      <p className="text-muted-foreground text-xs">{label}</p>
      <p className="tabular mt-0.5 text-sm font-medium">{value}</p>
    </Card>
  );
}
