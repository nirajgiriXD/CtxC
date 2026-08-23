/**
 * A project's real state, which is not just its status.
 *
 * A project whose directory is gone, and one that fell back to polling because
 * a watcher could not start, are both still "active" in the registry. Those are
 * exactly the cases worth surfacing, so the badge reads the registry and the
 * supervisor together rather than either alone.
 */

import type { Project, WatchReport } from "../lib/api";
import { Badge, Tooltip } from "./ui";

export function ProjectStatus({
  project,
  watch,
}: {
  project: Project;
  watch?: WatchReport;
}) {
  if (!project.exists) {
    return (
      <Tooltip label="The directory this project points at is no longer there.">
        <Badge variant="destructive">Missing</Badge>
      </Tooltip>
    );
  }
  if (project.status === "paused") {
    return (
      <Tooltip label="Registered, but left alone until resumed.">
        <Badge>Paused</Badge>
      </Tooltip>
    );
  }
  if (watch?.degraded_reason) {
    return (
      <Tooltip label={watch.degraded_reason}>
        <Badge variant="warning">Polling</Badge>
      </Tooltip>
    );
  }
  if (watch?.watching) {
    return (
      <Tooltip label="Changes are picked up as they happen.">
        <Badge variant="success">Watching</Badge>
      </Tooltip>
    );
  }
  return (
    <Tooltip label="Registered and active; the daemon has not reported on it yet.">
      <Badge variant="outline">Active</Badge>
    </Tooltip>
  );
}

/** The watch report for a project, which the daemon keys by name. */
export function watchOf(
  project: Project,
  watching: WatchReport[],
): WatchReport | undefined {
  return watching.find(
    (report) => report.project === project.name || report.path === project.path,
  );
}
