/**
 * Projects, managed through the same registry the CLI uses.
 *
 * Every action here is one API call, and the dashboard has no privileged path
 * around it: anything possible in this panel is possible from `ctxc project`,
 * and vice versa.
 */

import { useState } from "react";
import { FolderPlus, Pause, Play, RefreshCw, Trash2 } from "lucide-react";

import { api, RequestFailed, type Project, type WatchReport } from "../api";
import { ago, exact } from "../format";
import { useApi } from "../useApi";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Empty, Failure, Skeleton } from "./ui";

export function Projects({
  watching,
  revision,
  onSelect,
  selected,
}: {
  watching: WatchReport[];
  revision: number;
  onSelect: (project?: string) => void;
  selected?: string;
}) {
  const projects = useApi(() => api.projects(), [revision]);
  const [busy, setBusy] = useState<string | null>(null);
  const [failure, setFailure] = useState<RequestFailed | null>(null);

  /**
   * Run one registry action.
   *
   * The list is not updated optimistically: the daemon announces the change and
   * the reload that follows shows what actually happened, which matters when a
   * directory has been deleted underneath a project.
   */
  const act = async (id: string, action: () => Promise<unknown>) => {
    setBusy(id);
    setFailure(null);
    try {
      await action();
      projects.reload();
    } catch (cause) {
      setFailure(
        cause instanceof RequestFailed
          ? cause
          : new RequestFailed(0, String(cause)),
      );
    } finally {
      setBusy(null);
    }
  };

  if (projects.error) {
    return (
      <Failure
        message={projects.error.message}
        hint={"hint" in projects.error ? projects.error.hint : undefined}
        onRetry={projects.reload}
      />
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {failure ? (
        <Failure message={failure.message} hint={failure.hint} />
      ) : null}

      <AddProject onAdded={projects.reload} />

      <Card>
        <CardHeader>
          <CardTitle>Projects</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          {projects.loading && !projects.data ? (
            <>
              <Skeleton className="h-16" />
              <Skeleton className="h-16" />
            </>
          ) : !projects.data?.length ? (
            <Empty
              title="No projects registered yet."
              hint="Add one above, or run `ctxc project add <path>`."
            />
          ) : (
            projects.data.map((project) => (
              <Row
                key={project.id}
                project={project}
                watch={watching.find((report) => report.project === project.name)}
                busy={busy === project.id}
                selected={selected === project.id}
                onSelect={() =>
                  onSelect(selected === project.id ? undefined : project.id)
                }
                onPause={() => act(project.id, () => api.pauseProject(project.id))}
                onResume={() => act(project.id, () => api.resumeProject(project.id))}
                onReindex={() => act(project.id, () => api.reindexProject(project.id))}
                onRemove={() => act(project.id, () => api.removeProject(project.id))}
              />
            ))
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Row({
  project,
  watch,
  busy,
  selected,
  onSelect,
  onPause,
  onResume,
  onReindex,
  onRemove,
}: {
  project: Project;
  watch?: WatchReport;
  busy: boolean;
  selected: boolean;
  onSelect: () => void;
  onPause: () => void;
  onResume: () => void;
  onReindex: () => void;
  onRemove: () => void;
}) {
  const paused = project.status === "paused";

  return (
    <div
      className={[
        "flex flex-wrap items-center gap-3 rounded-md border p-3 transition-colors",
        selected ? "border-primary/50 bg-primary/5" : "border-border",
      ].join(" ")}
    >
      <button
        onClick={onSelect}
        className="min-w-0 flex-1 text-left"
        title={selected ? "Show every project" : "Show only this project"}
      >
        <span className="flex items-center gap-2">
          <Status project={project} watch={watch} />
          <span className="truncate text-sm font-medium">{project.name}</span>
        </span>
        {/* A path is repository content of a sort — it is whatever the user
            named their directories. React escapes it; it is never HTML. */}
        <span className="text-muted-foreground block truncate font-mono text-xs">
          {project.path}
        </span>
      </button>

      <div className="text-muted-foreground tabular hidden text-xs sm:block">
        <p>{exact(project.indexed_files)} files</p>
        <p>{exact(project.symbols)} symbols</p>
      </div>

      <div className="text-muted-foreground hidden text-xs md:block">
        {project.last_indexed_at ? (
          <p>Indexed {ago(project.last_indexed_at)}</p>
        ) : (
          <p>Never indexed</p>
        )}
        {watch?.pending_changes ? (
          <p>{exact(watch.pending_changes)} changes settling</p>
        ) : null}
      </div>

      <div className="flex gap-1">
        <Button
          size="icon"
          variant="ghost"
          disabled={busy}
          onClick={paused ? onResume : onPause}
          title={paused ? "Resume" : "Pause"}
        >
          {paused ? <Play /> : <Pause />}
        </Button>
        <Button
          size="icon"
          variant="ghost"
          disabled={busy}
          onClick={onReindex}
          title="Re-index now"
        >
          <RefreshCw className={busy ? "animate-spin" : ""} />
        </Button>
        <Button
          size="icon"
          variant="ghost"
          disabled={busy}
          onClick={onRemove}
          title="Stop looking after this project"
          className="hover:text-destructive"
        >
          <Trash2 />
        </Button>
      </div>
    </div>
  );
}

/**
 * A project's real state, which is not just its status.
 *
 * A project whose directory is gone, or one that fell back to polling because a
 * watcher could not start, is still "active" in the registry. Those are exactly
 * the cases worth surfacing.
 */
function Status({ project, watch }: { project: Project; watch?: WatchReport }) {
  if (!project.exists) {
    return <Badge variant="destructive">Missing</Badge>;
  }
  if (project.status === "paused") {
    return <Badge>Paused</Badge>;
  }
  if (watch?.degraded_reason) {
    return (
      <Badge variant="warning" title={watch.degraded_reason}>
        Polling
      </Badge>
    );
  }
  if (watch?.watching) {
    return <Badge variant="success">Watching</Badge>;
  }
  return <Badge>Active</Badge>;
}

function AddProject({ onAdded }: { onAdded: () => void }) {
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<RequestFailed | null>(null);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!path.trim()) return;

    setBusy(true);
    setFailure(null);
    try {
      await api.addProject(path.trim());
      setPath("");
      onAdded();
    } catch (cause) {
      setFailure(
        cause instanceof RequestFailed
          ? cause
          : new RequestFailed(0, String(cause)),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card>
      <CardContent className="p-4">
        <form onSubmit={submit} className="flex flex-wrap items-center gap-2">
          <FolderPlus className="text-muted-foreground size-4 shrink-0" />
          <input
            value={path}
            onChange={(event) => setPath(event.target.value)}
            placeholder="Path to a project directory"
            spellCheck={false}
            className="bg-background focus:ring-primary/50 min-w-64 flex-1 rounded-md border px-3 py-1.5 font-mono text-sm focus:outline-none focus:ring-2"
          />
          <Button type="submit" size="sm" disabled={busy || !path.trim()}>
            {busy ? "Adding…" : "Add project"}
          </Button>
        </form>
        {failure ? (
          <p className="text-destructive mt-2 text-xs">
            {failure.message}
            {failure.hint ? ` — ${failure.hint}` : ""}
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
