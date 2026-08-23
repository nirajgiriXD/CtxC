/**
 * What CtxC has saved, and what it is doing right now.
 *
 * The headline is deliberately not a single percentage. "62% smaller" is
 * trivia; a stage breakdown is something a person can act on, so it sits next
 * to the totals rather than three clicks away.
 */

import * as React from "react";
import {
  Activity as ActivityIcon,
  ArrowRight,
  Boxes,
  CircleDollarSign,
  Coins,
  Eye,
  Gauge,
  Timer,
  TriangleAlert,
} from "lucide-react";

import { api, type MetricEvent, type StageSavings, type Summary } from "../lib/api";
import type { ConnectionState, Revisions } from "../lib/events";
import { ago, compact, cost, duration, exact, percent, uptime } from "../lib/format";
import { href, Link } from "../lib/router";
import { hintOf, useApi, type Query } from "../lib/useApi";
import { ActivityList, mergeEvents } from "../components/activity-feed";
import { Sparkline } from "../components/charts";
import { PageBody, PageHeader, Stat, StatGrid } from "../components/page";
import { ProjectStatus, watchOf } from "../components/project-status";
import { useScope, WINDOWS } from "../components/scope";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardHeading,
  CardTitle,
  CardDescription,
  CardAction,
  Code,
  Empty,
  Failure,
  Meter,
  Notice,
  Skeleton,
  SkeletonRows,
} from "../components/ui";
import type { DaemonStatus } from "../lib/api";

export function Overview({
  revisions,
  status,
  recent,
  connection,
}: {
  revisions: Revisions;
  status: Query<DaemonStatus>;
  recent: MetricEvent[];
  connection: ConnectionState;
}) {
  const { days, project, selected, projects } = useScope();
  const window = WINDOWS.find((option) => option.days === days);

  const summary = useApi(
    () => api.summary(days, project),
    [days, project, revisions.operations],
  );
  const timeseries = useApi(
    () => api.timeseries(days <= 1 ? "hour" : "day", days, project),
    [days, project, revisions.operations],
  );
  const activity = useApi(
    () => api.activity(12, project),
    [project, revisions.operations],
  );

  const events = mergeEvents(recent, activity.data, project, 8);
  const spark = (timeseries.data?.points ?? []).map((point) => ({
    label: String(point.bucket_start),
    saved: point.tokens_saved,
  }));

  return (
    <PageBody>
      <PageHeader
        title="Overview"
        description={
          selected
            ? `${selected.name}, over the last ${window?.label ?? `${days} days`}.`
            : `Every project, over the last ${window?.label ?? `${days} days`}.`
        }
        actions={
          <Button variant="outline" size="sm" asChild>
            <Link to={href("/performance")}>
              Performance <ArrowRight />
            </Link>
          </Button>
        }
      />

      {status.error ? (
        <Failure
          message={status.error.message}
          hint={hintOf(status.error)}
          onRetry={status.reload}
        />
      ) : null}

      <DaemonFacts status={status} connection={connection} />

      {summary.error ? (
        <Failure
          message={summary.error.message}
          hint={hintOf(summary.error)}
          onRetry={summary.reload}
        />
      ) : summary.loading ? (
        <StatGrid>
          {Array.from({ length: 4 }, (_, index) => (
            <Skeleton key={index} className="h-[6.5rem]" />
          ))}
        </StatGrid>
      ) : summary.data && summary.data.operations === 0 ? (
        <Card>
          <Empty
            icon={Coins}
            title="Nothing recorded in this window yet."
            hint={
              <>
                Optimize something with <Code>ctxc optimize</Code>, search a
                project, or let the daemon index one. Numbers appear here as
                soon as anything runs.
              </>
            }
            action={
              <Button size="sm" asChild>
                <Link to={href("/projects")}>Add a project</Link>
              </Button>
            }
          />
        </Card>
      ) : summary.data ? (
        <Headline summary={summary.data} spark={spark} />
      ) : null}

      {summary.data && summary.data.operations > 0 ? (
        <Stages
          savings={summary.data.savings_by_stage}
          total={summary.data.tokens_saved}
        />
      ) : null}

      {/* Grid items default to a minimum width of their content, which a
          fixed-column activity row would push past the viewport on a phone.
          `min-w-0` lets them shrink and their own truncation take over. */}
      <div className="grid gap-6 xl:grid-cols-5">
        <Card className="min-w-0 xl:col-span-3">
          <CardHeader>
            <CardHeading>
              <CardTitle>Recent activity</CardTitle>
              <CardDescription>
                Streamed from the daemon as it happens.
              </CardDescription>
            </CardHeading>
            <CardAction>
              <Button variant="ghost" size="sm" asChild>
                <Link to={href("/activity")}>
                  See all <ArrowRight />
                </Link>
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent>
            {activity.loading && events.length === 0 ? (
              <SkeletonRows rows={5} height="h-9" />
            ) : (
              <ActivityList events={events} dense />
            )}
          </CardContent>
        </Card>

        <Card className="min-w-0 xl:col-span-2">
          <CardHeader>
            <CardHeading>
              <CardTitle>Projects</CardTitle>
              <CardDescription>
                {projects.length === 0
                  ? "None registered yet."
                  : `${projects.filter((entry) => entry.status === "active").length} of ${projects.length} active.`}
              </CardDescription>
            </CardHeading>
            <CardAction>
              <Button variant="ghost" size="sm" asChild>
                <Link to={href("/projects")}>
                  Manage <ArrowRight />
                </Link>
              </Button>
            </CardAction>
          </CardHeader>
          <CardContent>
            {projects.length === 0 ? (
              <Empty
                icon={Boxes}
                title="No projects registered."
                hint="Register one and the daemon keeps its index current."
                action={
                  <Button size="sm" asChild>
                    <Link to={href("/projects")}>Add a project</Link>
                  </Button>
                }
              />
            ) : (
              <ul className="divide-border/60 divide-y">
                {projects.slice(0, 6).map((entry) => (
                  <li key={entry.id}>
                    <Link
                      to={href("/projects", entry.id)}
                      className="hover:bg-muted/50 -mx-2 flex items-center gap-3 rounded-md px-2 py-2.5 transition-colors"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium">
                          {entry.name}
                        </span>
                        <span className="text-muted-foreground block truncate font-mono text-xs">
                          {entry.path}
                        </span>
                      </span>
                      <span className="text-muted-foreground tabular hidden text-xs sm:block">
                        {compact(entry.indexed_files)} files
                      </span>
                      <ProjectStatus
                        project={entry}
                        watch={watchOf(entry, status.data?.watch ?? [])}
                      />
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </PageBody>
  );
}

function DaemonFacts({
  status,
  connection,
}: {
  status: Query<DaemonStatus>;
  connection: ConnectionState;
}) {
  if (status.loading && !status.data) {
    return <Skeleton className="h-24" />;
  }
  if (!status.data) return null;

  const data = status.data;

  return (
    <div className="space-y-4">
      <Card>
        <CardContent className="divide-border/60 grid divide-y px-5 py-4 sm:grid-cols-2 sm:divide-y-0 lg:grid-cols-4">
          <Fact label="Daemon">
            <span className="inline-flex items-center gap-2">
              v{data.version}
              <Badge variant="outline" className="tabular">
                pid {data.pid}
              </Badge>
            </span>
          </Fact>
          <Fact label="Uptime" detail={`started ${ago(data.started_at)}`}>
            {uptime(data.uptime_ms)}
          </Fact>
          <Fact
            label="Observing"
            detail={
              data.degraded > 0
                ? `${data.degraded} scanning instead of watching`
                : "all watched projects have a watcher"
            }
          >
            {data.watching} of {data.active_projects} active
          </Fact>
          <Fact label="Indexed" detail="files across every project">
            {exact(data.indexed_files)}
          </Fact>
        </CardContent>
      </Card>

      {connection !== "live" ? (
        <Notice tone="warning" title="Not receiving live events">
          Numbers on this page are from the last successful read. The dashboard
          keeps trying to reconnect.
        </Notice>
      ) : null}
    </div>
  );
}

function Fact({
  label,
  detail,
  children,
}: {
  label: string;
  detail?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="py-2 sm:py-0">
      <p className="text-muted-foreground text-xs">{label}</p>
      <p className="tabular mt-1 text-sm font-medium">{children}</p>
      {detail ? (
        <p className="text-muted-foreground mt-0.5 text-xs">{detail}</p>
      ) : null}
    </div>
  );
}

function Headline({
  summary,
  spark,
}: {
  summary: Summary;
  spark: { label: string; saved: number }[];
}) {
  const amount = cost(summary.estimated_cost_saved);
  const savedAnything = spark.some((point) => point.saved !== 0);

  return (
    <StatGrid>
      <Card className="overflow-hidden">
        <CardContent className="px-5 py-4">
          <p className="text-muted-foreground text-xs font-medium">Tokens saved</p>
          <p className="tabular text-primary mt-1.5 text-2xl font-semibold tracking-tight">
            {compact(summary.tokens_saved)}
          </p>
          <p className="text-muted-foreground mt-1 text-xs">
            {compact(summary.input_tokens)} in, {compact(summary.output_tokens)} out
          </p>
          {savedAnything ? (
            <div className="mt-2 -mb-1">
              <Sparkline data={spark} dataKey="saved" />
            </div>
          ) : null}
        </CardContent>
      </Card>

      <Stat
        label="Reduction"
        value={percent(summary.reduction_ratio)}
        detail={`${exact(summary.operations)} operations`}
        icon={Gauge}
      />

      <Stat
        label="Estimated cost saved"
        value={amount ?? "Not estimated"}
        detail={
          amount
            ? `at ${summary.estimated_cost_saved?.model} rates`
            : "set metrics.cost_per_million_input_tokens in Settings"
        }
        icon={CircleDollarSign}
      />

      <Stat
        label="Latency"
        value={
          summary.average_duration_ms === undefined
            ? "—"
            : duration(summary.average_duration_ms)
        }
        detail={
          summary.slowest_duration_ms > 0
            ? `${duration(summary.slowest_duration_ms)} slowest`
            : "average per operation"
        }
        icon={Timer}
      />

      <Stat
        label="Cache hit rate"
        value={
          summary.cache_hit_rate === undefined
            ? "—"
            : percent(summary.cache_hit_rate)
        }
        detail={
          summary.cache_hit_rate === undefined
            ? "nothing used a cache in this window"
            : "index reads that were already current"
        }
        icon={Eye}
      />

      <Stat
        label="Errors"
        value={exact(summary.errors)}
        detail={
          summary.degradations > 0
            ? `${exact(summary.degradations)} degraded`
            : "nothing degraded"
        }
        tone={summary.errors > 0 ? "bad" : undefined}
        icon={TriangleAlert}
      />

      <Stat
        label="Operations"
        value={exact(summary.operations)}
        detail="in this window"
        icon={ActivityIcon}
      />

      <Stat
        label="Token counts"
        value={summary.estimated ? "Estimated" : "Exact"}
        detail={
          summary.estimated
            ? "counted by heuristic, not the target tokenizer"
            : "counted by the target tokenizer"
        }
        icon={Coins}
      />
    </StatGrid>
  );
}

/**
 * Where the reduction actually came from.
 *
 * Its own card, header included, so it can be dropped into a page without a
 * second card around it — which is what would happen if the caller had to
 * supply the heading.
 */
export function Stages({
  savings,
  total,
}: {
  savings: StageSavings;
  total: number;
}) {
  const rows = [
    ["Filtering", savings.filtering],
    ["Deduplication", savings.deduplication],
    ["Compression", savings.compression],
    ["Relevance selection", savings.selection],
  ] as const;

  const widest = Math.max(...rows.map(([, value]) => Math.abs(value)), 1);

  return (
    <Card>
      <CardHeader>
        <CardHeading>
          <CardTitle>Savings by stage</CardTitle>
          <CardDescription>
            Which part of the pipeline earned the reduction.
          </CardDescription>
        </CardHeading>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-center gap-4 text-sm">
            <span className="text-muted-foreground w-40 shrink-0 text-xs">
              {label}
            </span>
            {/* A stage can cost tokens rather than save them; collapsing
                repeated log lines and noting how many adds a few back. */}
            <Meter
              value={value}
              max={widest}
              tone={value < 0 ? "warning" : "primary"}
              className="flex-1"
            />
            <span className="tabular w-28 shrink-0 text-right text-xs">
              {value < 0 ? `+${compact(-value)} added` : compact(value)}
            </span>
          </div>
        ))}
        <p className="text-muted-foreground text-xs">
          Adds up to {compact(total)} tokens saved.
        </p>
      </CardContent>
    </Card>
  );
}
