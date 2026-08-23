/**
 * The daemon, the storage, and what it has been saying.
 *
 * This is `ctxc status` with more room: the same facts, from the same places, so
 * a person comparing the two never has to wonder which one is right. The log
 * panel is the part a terminal cannot give you — a daemon started with
 * `--detach` has no console, and its recent records are kept in memory for
 * exactly this.
 */

import * as React from "react";
import {
  CircleSlash,
  Database,
  FileClock,
  HardDrive,
  Network,
  Radio,
  Server,
} from "lucide-react";

import {
  api,
  type DaemonStatus,
  type LogRecord,
  type ProcessList,
} from "../lib/api";
import { bytes, clock, exact, stamp, uptime } from "../lib/format";
import { cn } from "../lib/utils";
import { hintOf, useApi, useMutation, type Query } from "../lib/useApi";
import {
  Detail,
  DetailList,
  PageBody,
  PageHeader,
  Section,
  Stat,
  StatGrid,
} from "../components/page";
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
  Empty,
  Failure,
  Notice,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  SkeletonRows,
  Switch,
  toast,
  Tooltip,
} from "../components/ui";

export function System({
  revision,
  status,
}: {
  revision: number;
  status: Query<DaemonStatus>;
}) {
  const diagnostics = useApi(() => api.diagnostics(), [revision]);
  const processes = useApi(() => api.processes(), [revision]);

  return (
    <PageBody>
      <PageHeader
        title="System"
        description="What is running, where it keeps things, and what it has been saying."
        actions={<StopDaemon processes={processes} />}
      />

      {diagnostics.error ? (
        <Failure
          message={diagnostics.error.message}
          hint={hintOf(diagnostics.error)}
          onRetry={diagnostics.reload}
        />
      ) : null}

      {diagnostics.loading && !diagnostics.data ? (
        <SkeletonRows rows={4} height="h-24" />
      ) : diagnostics.data ? (
        <>
          <StatGrid>
            <Stat
              label="CtxC"
              value={`v${diagnostics.data.version}`}
              detail={`${diagnostics.data.os} · ${diagnostics.data.arch}`}
              icon={Server}
            />
            <Stat
              label="Uptime"
              value={uptime(diagnostics.data.uptime_ms)}
              detail={`pid ${diagnostics.data.pid}, since ${stamp(diagnostics.data.started_at)}`}
              icon={Radio}
            />
            <Stat
              label="Database"
              value={
                diagnostics.data.database.size_bytes === undefined
                  ? "—"
                  : bytes(diagnostics.data.database.size_bytes)
              }
              detail={`schema v${diagnostics.data.database.schema_version}`}
              icon={Database}
            />
            <Stat
              label="Stored context"
              value={exact(diagnostics.data.contexts)}
              detail={`${exact(diagnostics.data.indexed_roots)} indexed roots`}
              icon={HardDrive}
            />
          </StatGrid>

          <OtherProcesses processes={processes} />

          <div className="grid gap-6 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardHeading>
                  <CardTitle>Where things are</CardTitle>
                  <CardDescription>
                    Resolved once at startup, from the platform conventions and{" "}
                    <Code>CTXC_HOME</Code>.
                  </CardDescription>
                </CardHeading>
              </CardHeader>
              <CardContent>
                <DetailList>
                  <Detail label="Configuration" mono>
                    {diagnostics.data.locations.config_file}
                    {diagnostics.data.config_file_exists ? null : (
                      <span className="text-muted-foreground ml-2 font-sans text-xs">
                        (not created yet)
                      </span>
                    )}
                  </Detail>
                  <Detail label="Database" mono>
                    {diagnostics.data.database.path}
                  </Detail>
                  <Detail label="Data directory" mono>
                    {diagnostics.data.locations.data_dir}
                  </Detail>
                  <Detail label="Cache directory" mono>
                    {diagnostics.data.locations.cache_dir}
                  </Detail>
                </DetailList>
              </CardContent>
            </Card>

            <WatchPanel status={status} />
          </div>

          <LogPanel held={diagnostics.data.logs} revision={revision} />

          <Section
            title="API surface"
            description="What this build answers. The CLI and this dashboard both go through exactly these."
          >
            <Card>
              <CardContent className="px-5 py-4">
                <ul className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3">
                  {diagnostics.data.routes.map((route) => (
                    <li
                      key={route}
                      className="text-muted-foreground truncate font-mono text-xs"
                    >
                      {route}
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>
          </Section>
        </>
      ) : null}
    </PageBody>
  );
}

function WatchPanel({ status }: { status: Query<DaemonStatus> }) {
  const reports = status.data?.watch ?? [];

  return (
    <Card>
      <CardHeader>
        <CardHeading>
          <CardTitle>Observation</CardTitle>
          <CardDescription>
            Filesystem watchers where the platform allows them, periodic scanning
            where it does not.
          </CardDescription>
        </CardHeading>
        {status.data ? (
          <Badge variant={status.data.degraded > 0 ? "warning" : "success"}>
            {status.data.watching} watched
            {status.data.degraded > 0 ? `, ${status.data.degraded} polling` : ""}
          </Badge>
        ) : null}
      </CardHeader>
      <CardContent>
        {reports.length === 0 ? (
          <Empty
            icon={Network}
            title="Nothing is being observed."
            hint="Register a project, or resume a paused one."
          />
        ) : (
          <ul className="divide-border/60 divide-y">
            {reports.map((report) => (
              <li key={report.path} className="space-y-1 py-2.5">
                <div className="flex items-center gap-3">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {report.project}
                  </span>
                  {report.pending_changes > 0 ? (
                    <Badge variant="info">
                      {exact(report.pending_changes)} settling
                    </Badge>
                  ) : null}
                  <Badge variant={report.watching ? "success" : "warning"}>
                    {report.watching ? "Watching" : "Polling"}
                  </Badge>
                </div>
                <p className="text-muted-foreground truncate font-mono text-xs">
                  {report.path}
                </p>
                {report.degraded_reason ? (
                  <p className="text-warning text-xs">{report.degraded_reason}</p>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

// ------------------------------------------------------------------- logs

const LEVELS = ["info", "warn", "error", "debug", "trace"] as const;

const LEVEL_TONE: Record<string, string> = {
  ERROR: "text-destructive",
  WARN: "text-warning",
  INFO: "text-info",
  DEBUG: "text-muted-foreground",
  TRACE: "text-muted-foreground",
};

function LogPanel({
  held,
  revision,
}: {
  held: { held: number; dropped: number; capacity: number };
  revision: number;
}) {
  const [level, setLevel] = React.useState<string>("info");
  const [follow, setFollow] = React.useState(true);
  const bottom = React.useRef<HTMLDivElement | null>(null);

  const logs = useApi(
    () => api.logs(300, level),
    // Following means re-reading whenever the daemon does anything at all.
    // Not following pins the list: the revision is dropped from the
    // dependencies rather than the dependencies changing shape, which React
    // does not allow between renders.
    [level, follow ? revision : 0],
  );

  React.useEffect(() => {
    if (follow) bottom.current?.scrollIntoView({ block: "nearest" });
  }, [follow, logs.data]);

  return (
    <Section
      title="Recent log"
      description="Kept in memory by the running daemon. Nothing is written to disk, and the oldest records fall out to make room."
    >
      <Card>
        <CardHeader>
          <CardHeading>
            <CardTitle>
              {exact(held.held)} of {exact(held.capacity)} records held
            </CardTitle>
            <CardDescription>
              {held.dropped > 0
                ? `${exact(held.dropped)} older records have fallen out of the buffer.`
                : "Nothing has fallen out of the buffer yet."}
            </CardDescription>
          </CardHeading>
          <CardAction>
            <label className="flex items-center gap-2 text-xs">
              <Switch checked={follow} onCheckedChange={setFollow} />
              Follow
            </label>
            <Select value={level} onValueChange={setLevel}>
              <SelectTrigger size="sm" className="w-28" aria-label="Level">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LEVELS.map((name) => (
                  <SelectItem key={name} value={name}>
                    {name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </CardAction>
        </CardHeader>
        <CardContent>
          {logs.error ? (
            <Failure
              message={logs.error.message}
              hint={hintOf(logs.error)}
              onRetry={logs.reload}
            />
          ) : logs.loading && !logs.data ? (
            <SkeletonRows rows={8} height="h-5" />
          ) : logs.data && logs.data.records.length > 0 ? (
            <ScrollArea className="bg-muted/30 h-96 rounded-md border">
              <div className="p-3 font-mono text-xs">
                {logs.data.records.map((record) => (
                  <LogLine key={record.seq} record={record} />
                ))}
                <div ref={bottom} />
              </div>
            </ScrollArea>
          ) : (
            <Empty
              icon={FileClock}
              title={`Nothing at ${level} or above.`}
              hint="The daemon is quiet when it has nothing to report. Lower the level to see more."
            />
          )}
        </CardContent>
      </Card>
    </Section>
  );
}

function LogLine({ record }: { record: LogRecord }) {
  const fields = Object.entries(record.fields ?? {});

  return (
    <div className="flex gap-2 py-0.5 leading-relaxed">
      <Tooltip label={stamp(record.at)}>
        <span className="text-muted-foreground shrink-0">{clock(record.at)}</span>
      </Tooltip>
      <span
        className={cn("w-12 shrink-0 font-medium", LEVEL_TONE[record.level])}
      >
        {record.level}
      </span>
      <span className="text-muted-foreground hidden w-40 shrink-0 truncate md:block">
        {record.target}
      </span>
      <span className="min-w-0 flex-1 break-words">
        {record.message}
        {fields.map(([name, value]) => (
          <span key={name} className="text-muted-foreground ml-2">
            {name}={value}
          </span>
        ))}
      </span>
    </div>
  );
}

// ------------------------------------------------------------------- stop

/**
 * The label says "daemon and dashboard" rather than "CtxC" because that is all
 * it stops.
 *
 * The daemon serves this page and the API, and it watches projects. It is not
 * the only CtxC process: an agent with CtxC configured spawns `ctxc mcp`, and
 * those keep running. The dashboard cannot end them — a web page must not be
 * able to terminate processes on the machine serving it — so it names them and
 * says which command does.
 */
function StopDaemon({ processes }: { processes: Query<ProcessList> }) {
  const [confirming, setConfirming] = React.useState(false);
  const others = processes.data?.processes.filter((one) => !one.is_daemon) ?? [];

  const stop = useMutation(() => api.shutdown(), {
    onDone: (result) => {
      setConfirming(false);
      toast.success("The daemon and dashboard are stopping.", {
        description:
          result.still_running > 0
            ? `${count(result.still_running)} still running. Run \`${result.stop_command}\` to end everything.`
            : "Start it again with `ctxc start --detach`.",
      });
    },
  });

  return (
    <>
      {stop.error ? (
        <Notice tone="warning" title="Could not stop the daemon">
          {stop.error.message}
        </Notice>
      ) : null}
      <Button variant="danger" size="sm" onClick={() => setConfirming(true)}>
        <CircleSlash /> Stop daemon &amp; dashboard
      </Button>
      <Confirm
        open={confirming}
        onOpenChange={setConfirming}
        destructive
        pending={stop.pending}
        title="Stop the daemon and dashboard?"
        description={
          <span className="space-y-2 block">
            <span className="block">
              This page is served by the daemon, so it will stop responding.
              Nothing is deleted, and <Code>ctxc start --detach</Code> brings it
              back.
            </span>
            {others.length > 0 ? (
              <span className="block">
                It does not stop everything. {count(others.length)} would keep
                running:{" "}
                {others.map((one) => `ctxc ${one.command} (pid ${one.pid})`).join(", ")}.
                Run <Code>ctxc stop</Code> in a terminal to end those too.
              </span>
            ) : (
              <span className="block">
                Nothing else CtxC is running right now, so this stops all of it.
              </span>
            )}
          </span>
        }
        confirmLabel="Stop it"
        onConfirm={() => void stop.run()}
      />
    </>
  );
}

/** "1 other process" / "3 other processes", so the sentence reads. */
function count(many: number) {
  return `${many} other process${many === 1 ? "" : "es"}`;
}

/**
 * What else CtxC is running, and the one command that ends all of it.
 *
 * Listed on this page rather than only in the confirmation dialog: someone
 * wondering why `ctxc` is still in their task manager after pressing Stop
 * should be able to find the answer without pressing it again.
 */
function OtherProcesses({ processes }: { processes: Query<ProcessList> }) {
  const others = processes.data?.processes.filter((one) => !one.is_daemon) ?? [];

  return (
    <Section
      title="Also running"
      description="CtxC processes besides the daemon. The dashboard cannot stop these."
    >
      {processes.error ? (
        <Failure
          message={processes.error.message}
          hint={hintOf(processes.error)}
          onRetry={processes.reload}
        />
      ) : processes.loading && !processes.data ? (
        <SkeletonRows rows={2} height="h-10" />
      ) : others.length === 0 ? (
        <Empty
          title="Nothing else is running"
          hint="Only the daemon. Stopping it stops all of CtxC."
        />
      ) : (
        <Card>
          <CardContent>
            <DetailList>
              {others.map((one) => (
                <Detail key={one.pid} label={`ctxc ${one.command}`}>
                  <span className="tabular">pid {one.pid}</span>
                  <span className="text-muted-foreground ml-2">
                    started {stamp(one.started_at)}
                  </span>
                </Detail>
              ))}
            </DetailList>
            <Notice className="mt-4" title="How to stop everything">
              Pressing <strong>Stop daemon &amp; dashboard</strong> leaves these
              running. To end the daemon and all of them at once, run{" "}
              <Code>ctxc stop</Code> in a terminal. Add{" "}
              <Code>--all</Code> to reach CtxC processes under a different{" "}
              <Code>CTXC_HOME</Code>.
            </Notice>
          </CardContent>
        </Card>
      )}
    </Section>
  );
}
