/**
 * Projects, managed through the same registry the CLI uses.
 *
 * Every action here is one API call, and the dashboard has no privileged path
 * around it: anything possible in this panel is possible from `ctxc project`,
 * and anything possible there is possible here.
 *
 * Nothing is applied optimistically. The daemon announces the change and the
 * re-read that follows shows what actually happened, which matters when a
 * directory has been deleted underneath a project.
 */

import * as React from "react";
import {
  ArrowLeft,
  Boxes,
  Clock,
  FileCode,
  FolderPlus,
  MoreHorizontal,
  Network,
  Pause,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";

import { api, type DaemonStatus, type Project } from "../lib/api";
import type { Revisions } from "../lib/events";
import { ago, compact, duration, exact, stamp } from "../lib/format";
import { href, Link, navigate } from "../lib/router";
import { hintOf, useApi, useMutation } from "../lib/useApi";
import { RankChart } from "../components/charts";
import {
  Detail,
  DetailList,
  PageBody,
  PageHeader,
  Section,
  Stat,
  StatGrid,
} from "../components/page";
import { ProjectStatus, watchOf } from "../components/project-status";
import { useScope } from "../components/scope";
import {
  Badge,
  Button,
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardHeading,
  CardTitle,
  Code,
  Confirm,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Empty,
  Failure,
  Field,
  Input,
  Notice,
  SkeletonRows,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  toast,
  Tooltip,
} from "../components/ui";

export function Projects({
  selected,
  revisions,
  status,
}: {
  selected?: string;
  revisions: Revisions;
  status: DaemonStatus | null;
}) {
  return selected ? (
    <ProjectDetail id={selected} revisions={revisions} status={status} />
  ) : (
    <ProjectList revisions={revisions} status={status} />
  );
}

// ------------------------------------------------------------------- list

function ProjectList({
  revisions,
  status,
}: {
  revisions: Revisions;
  status: DaemonStatus | null;
}) {
  const projects = useApi(() => api.projects(), [revisions.projects]);
  const [adding, setAdding] = React.useState(false);

  return (
    <PageBody>
      <PageHeader
        title="Projects"
        description="Register a directory and the daemon keeps its index current, watching for changes where the platform allows it."
        actions={
          <Button size="sm" onClick={() => setAdding(true)}>
            <FolderPlus /> Add project
          </Button>
        }
      />

      {projects.error ? (
        <Failure
          message={projects.error.message}
          hint={hintOf(projects.error)}
          onRetry={projects.reload}
        />
      ) : null}

      {status && status.degraded > 0 ? (
        <Notice tone="warning" title="Some projects are being scanned, not watched">
          A filesystem watcher could not be started for {status.degraded} of them,
          so changes are found by scanning instead. Open a project to see why.
        </Notice>
      ) : null}

      <Card>
        <CardContent className="px-0 pb-0">
          {projects.loading && !projects.data ? (
            <div className="p-5">
              <SkeletonRows rows={3} height="h-12" />
            </div>
          ) : !projects.data?.length ? (
            <Empty
              icon={Boxes}
              title="No projects registered yet."
              hint={
                <>
                  Add one here, or run <Code>ctxc project add &lt;path&gt;</Code>{" "}
                  in a terminal. Either way it lands in the same registry.
                </>
              }
              action={
                <Button size="sm" onClick={() => setAdding(true)}>
                  <FolderPlus /> Add project
                </Button>
              }
            />
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  {/* The project column takes whatever is left; the rest are
                      sized to their contents so a long path never squeezes the
                      numbers into two lines. */}
                  <TableHead className="w-full pl-5">Project</TableHead>
                  <TableHead className="w-px">Status</TableHead>
                  <TableHead numeric className="w-px">
                    Files
                  </TableHead>
                  <TableHead numeric className="w-px">
                    Symbols
                  </TableHead>
                  <TableHead className="w-px">Last indexed</TableHead>
                  <TableHead className="w-px pr-5" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {projects.data.map((project) => (
                  <Row
                    key={project.id}
                    project={project}
                    watching={status?.watch ?? []}
                    onChanged={projects.reload}
                  />
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <AddProject
        open={adding}
        onOpenChange={setAdding}
        onAdded={projects.reload}
      />
    </PageBody>
  );
}

function Row({
  project,
  watching,
  onChanged,
}: {
  project: Project;
  watching: DaemonStatus["watch"];
  onChanged: () => void;
}) {
  const actions = useProjectActions(project, onChanged);

  return (
    <>
      <TableRow>
        <TableCell className="max-w-0 pl-5">
          <Link
            to={href("/projects", project.id)}
            className="block min-w-0 hover:underline"
          >
            <span className="block truncate font-medium">{project.name}</span>
            {/* A path is repository content of a sort — it is whatever the user
                named their directories. React escapes it; it is never HTML. */}
            <span className="text-muted-foreground block truncate font-mono text-xs">
              {project.path}
            </span>
          </Link>
        </TableCell>
        <TableCell>
          <ProjectStatus project={project} watch={watchOf(project, watching)} />
        </TableCell>
        <TableCell numeric className="text-muted-foreground">
          {exact(project.indexed_files)}
        </TableCell>
        <TableCell numeric className="text-muted-foreground">
          {exact(project.symbols)}
        </TableCell>
        <TableCell className="text-muted-foreground text-xs whitespace-nowrap">
          {project.last_indexed_at ? (
            <Tooltip label={stamp(project.last_indexed_at)}>
              <span>{ago(project.last_indexed_at)}</span>
            </Tooltip>
          ) : (
            "Never"
          )}
        </TableCell>
        <TableCell className="pr-5">
          <ProjectMenu project={project} actions={actions} />
        </TableCell>
      </TableRow>
      {actions.dialogs}
    </>
  );
}

/** The actions a project has, wherever it is shown. */
function useProjectActions(project: Project, onChanged: () => void) {
  const [removing, setRemoving] = React.useState(false);

  const announce = (message: string) => {
    toast.success(message);
    onChanged();
  };

  const pause = useMutation(() => api.pauseProject(project.id), {
    onDone: () => announce(`Paused ${project.name}.`),
  });
  const resume = useMutation(() => api.resumeProject(project.id), {
    onDone: () => announce(`Resumed ${project.name}.`),
  });
  const reindex = useMutation(() => api.reindexProject(project.id), {
    onDone: (report) =>
      announce(
        `Re-indexed ${project.name}: ${exact(report.indexed)} files in ${duration(report.duration_ms)}.`,
      ),
  });
  const remove = useMutation(() => api.removeProject(project.id), {
    onDone: () => {
      setRemoving(false);
      toast.success(`Removed ${project.name}.`, {
        description: "Its files were not touched.",
      });
      onChanged();
      navigate(href("/projects"));
    },
  });

  const failure =
    pause.error ?? resume.error ?? reindex.error ?? remove.error ?? null;

  const dialogs = (
    <Confirm
      open={removing}
      onOpenChange={setRemoving}
      destructive
      pending={remove.pending}
      title={`Remove ${project.name}?`}
      description="CtxC forgets the project and its index. The directory and everything in it is left exactly as it is."
      confirmLabel="Remove it"
      onConfirm={() => void remove.run()}
    />
  );

  return {
    project,
    pause,
    resume,
    reindex,
    remove,
    failure,
    askToRemove: () => setRemoving(true),
    busy: pause.pending || resume.pending || reindex.pending || remove.pending,
    dialogs,
  };
}

type ProjectActions = ReturnType<typeof useProjectActions>;

function ProjectMenu({
  project,
  actions,
}: {
  project: Project;
  actions: ProjectActions;
}) {
  const paused = project.status === "paused";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          disabled={actions.busy}
          aria-label={`Actions for ${project.name}`}
        >
          <MoreHorizontal />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-44">
        <DropdownMenuItem asChild>
          <Link to={href("/projects", project.id)}>
            <FileCode /> Open
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() =>
            void (paused ? actions.resume.run() : actions.pause.run())
          }
        >
          {paused ? <Play /> : <Pause />}
          {paused ? "Resume monitoring" : "Pause monitoring"}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void actions.reindex.run()}>
          <RefreshCw /> Re-index now
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={actions.askToRemove}>
          <Trash2 /> Remove
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AddProject({
  open,
  onOpenChange,
  onAdded,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdded: () => void;
}) {
  const [path, setPath] = React.useState("");

  const add = useMutation((directory: string) => api.addProject(directory), {
    onDone: (project) => {
      toast.success(`Added ${project.name}.`, {
        description: "The daemon will index it shortly.",
      });
      setPath("");
      onOpenChange(false);
      onAdded();
    },
  });

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const trimmed = path.trim();
    if (trimmed) void add.run(trimmed);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) add.clearError();
        onOpenChange(next);
      }}
    >
      <DialogContent>
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>Add a project</DialogTitle>
            <DialogDescription>
              The path is resolved on the machine the daemon is running on, not
              in this browser — so it is an absolute path on that machine.
            </DialogDescription>
          </DialogHeader>

          <Field
            label="Project directory"
            htmlFor="project-path"
            description="CtxC detects the languages, frameworks and package manager it finds there."
            error={
              add.error ? (
                <>
                  {add.error.message}
                  {hintOf(add.error) ? ` — ${hintOf(add.error)}` : ""}
                </>
              ) : undefined
            }
          >
            <Input
              id="project-path"
              value={path}
              onChange={(event) => setPath(event.target.value)}
              placeholder="/home/you/code/acme"
              spellCheck={false}
              autoFocus
              className="font-mono"
              aria-invalid={add.error ? true : undefined}
            />
          </Field>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={add.pending || !path.trim()}>
              {add.pending ? "Adding…" : "Add project"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ----------------------------------------------------------------- detail

function ProjectDetail({
  id,
  revisions,
  status,
}: {
  id: string;
  revisions: Revisions;
  status: DaemonStatus | null;
}) {
  const { setProject } = useScope();
  const project = useApi(() => api.project(id), [id, revisions.projects]);
  const metrics = useApi(
    () => api.summary(30, id),
    [id, revisions.operations],
  );
  const graph = useApi(() => api.projectGraph(id, 8), [id, revisions.operations]);

  if (project.error) {
    return (
      <PageBody>
        <PageHeader title="Project" />
        <Failure
          message={project.error.message}
          hint={hintOf(project.error)}
          onRetry={project.reload}
        />
        <div>
          <Button variant="outline" size="sm" asChild>
            <Link to={href("/projects")}>
              <ArrowLeft /> Back to projects
            </Link>
          </Button>
        </div>
      </PageBody>
    );
  }

  if (!project.data) {
    return (
      <PageBody>
        <SkeletonRows rows={4} height="h-20" />
      </PageBody>
    );
  }

  return (
    <ProjectDetailBody
      project={project.data}
      status={status}
      onChanged={project.reload}
      metrics={metrics}
      graph={graph}
      onScope={() => setProject(project.data!.id)}
    />
  );
}

function ProjectDetailBody({
  project,
  status,
  onChanged,
  metrics,
  graph,
  onScope,
}: {
  project: Project;
  status: DaemonStatus | null;
  onChanged: () => void;
  metrics: ReturnType<typeof useApi<Awaited<ReturnType<typeof api.summary>>>>;
  graph: ReturnType<typeof useApi<Awaited<ReturnType<typeof api.projectGraph>>>>;
  onScope: () => void;
}) {
  const actions = useProjectActions(project, onChanged);
  const watch = watchOf(project, status?.watch ?? []);
  const paused = project.status === "paused";
  const detection = project.detection;

  return (
    <PageBody>
      <PageHeader
        title={
          <span className="flex flex-wrap items-center gap-3">
            {project.name}
            <ProjectStatus project={project} watch={watch} />
          </span>
        }
        description={<span className="font-mono text-xs">{project.path}</span>}
        actions={
          <>
            <Button variant="outline" size="sm" asChild>
              <Link to={href("/projects")}>
                <ArrowLeft /> All projects
              </Link>
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={actions.busy}
              onClick={() =>
                void (paused ? actions.resume.run() : actions.pause.run())
              }
            >
              {paused ? <Play /> : <Pause />}
              {paused ? "Resume" : "Pause"}
            </Button>
            <Button
              size="sm"
              disabled={actions.busy}
              onClick={() => void actions.reindex.run()}
            >
              <RefreshCw className={actions.reindex.pending ? "animate-spin" : ""} />
              Re-index
            </Button>
            <Button
              variant="danger"
              size="sm"
              disabled={actions.busy}
              onClick={actions.askToRemove}
            >
              <Trash2 /> Remove
            </Button>
          </>
        }
      />

      {actions.failure ? (
        <Failure
          message={actions.failure.message}
          hint={hintOf(actions.failure)}
        />
      ) : null}

      {!project.exists ? (
        <Notice tone="warning" title="The directory is gone">
          CtxC still holds this project's index, but nothing is there to read.
          Remove it, or restore the directory and re-index.
        </Notice>
      ) : null}

      {watch?.degraded_reason ? (
        <Notice tone="warning" title="Scanning instead of watching">
          {watch.degraded_reason}. Changes are still picked up, on the interval
          set by <Code>watch.poll_interval_ms</Code>.
        </Notice>
      ) : null}

      <StatGrid>
        <Stat label="Indexed files" value={exact(project.indexed_files)} icon={FileCode} />
        <Stat label="Symbols" value={exact(project.symbols)} icon={Network} />
        <Stat
          label="Last indexed"
          value={project.last_indexed_at ? ago(project.last_indexed_at) : "Never"}
          detail={
            project.last_indexed_at ? stamp(project.last_indexed_at) : undefined
          }
          icon={Clock}
        />
        <Stat
          label="Tokens saved"
          value={
            metrics.data ? compact(metrics.data.tokens_saved) : "—"
          }
          detail="over the last 30 days"
          tone="primary"
        />
      </StatGrid>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardHeading>
              <CardTitle>Registration</CardTitle>
              <CardDescription>
                What the registry holds, and what detection found.
              </CardDescription>
            </CardHeading>
            <CardAction>
              <Button variant="outline" size="sm" onClick={onScope}>
                Scope the dashboard to this
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent>
            <DetailList>
              <Detail label="Identifier" mono>
                {project.id}
              </Detail>
              <Detail label="Path" mono>
                {project.path}
              </Detail>
              <Detail label="Registered">{stamp(project.added_at)}</Detail>
              <Detail label="Version control">
                {detection.git ? "Git" : "None detected"}
              </Detail>
              <Detail label="Package manager">
                {detection.package_manager ?? "None detected"}
              </Detail>
              <Detail label="Languages">
                {detection.languages.length > 0 ? (
                  <span className="flex flex-wrap justify-end gap-1">
                    {detection.languages.map((language) => (
                      <Badge key={language} variant="outline">
                        {language}
                      </Badge>
                    ))}
                  </span>
                ) : (
                  "None detected"
                )}
              </Detail>
              <Detail label="Frameworks">
                {detection.frameworks.length > 0 ? (
                  <span className="flex flex-wrap justify-end gap-1">
                    {detection.frameworks.map((framework) => (
                      <Badge key={framework} variant="outline">
                        {framework}
                      </Badge>
                    ))}
                  </span>
                ) : (
                  "None detected"
                )}
              </Detail>
              <Detail label="Changes settling">
                {watch ? exact(watch.pending_changes) : "—"}
              </Detail>
            </DetailList>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardHeading>
              <CardTitle>Most depended on</CardTitle>
              <CardDescription>
                The files the rest of this project leans on. These are what a
                question about it usually needs.
              </CardDescription>
            </CardHeading>
            <CardAction>
              <Button variant="ghost" size="sm" asChild>
                <Link to={href("/context")}>Search this project</Link>
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent>
            {graph.loading && !graph.data ? (
              <SkeletonRows rows={4} height="h-8" />
            ) : graph.data && graph.data.most_depended_on.length > 0 ? (
              <>
                <RankChart
                  data={graph.data.most_depended_on.map((entry) => ({
                    label: entry.path,
                    value: entry.dependents,
                  }))}
                  format={(value) => exact(value)}
                />
                <p className="text-muted-foreground mt-3 text-xs">
                  {exact(graph.data.files)} files, {exact(graph.data.edges)}{" "}
                  resolved relationships.
                </p>
              </>
            ) : (
              <Empty
                icon={Network}
                title="No dependency graph yet."
                hint="Index the project, and relationships CtxC can resolve appear here."
              />
            )}
          </CardContent>
        </Card>
      </div>

      <Section
        title="Recent savings"
        description="From the same metrics `ctxc status --metrics --project` reports."
      >
        <Card>
          <CardContent className="px-5 py-4">
            {metrics.error ? (
              <Failure
                message={metrics.error.message}
                hint={hintOf(metrics.error)}
                onRetry={metrics.reload}
              />
            ) : metrics.data && metrics.data.operations > 0 ? (
              <DetailList>
                <Detail label="Operations">{exact(metrics.data.operations)}</Detail>
                <Detail label="Tokens in">
                  {exact(metrics.data.input_tokens)}
                </Detail>
                <Detail label="Tokens out">
                  {exact(metrics.data.output_tokens)}
                </Detail>
                <Detail label="Average duration">
                  {metrics.data.average_duration_ms === undefined
                    ? "—"
                    : duration(metrics.data.average_duration_ms)}
                </Detail>
                <Detail label="Errors">{exact(metrics.data.errors)}</Detail>
              </DetailList>
            ) : (
              <Empty
                title="Nothing recorded for this project in the last 30 days."
                hint="Indexing, searching and optimizing all leave a record here."
              />
            )}
          </CardContent>
        </Card>
      </Section>

      {actions.dialogs}
    </PageBody>
  );
}
