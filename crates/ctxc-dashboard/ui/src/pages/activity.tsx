/**
 * The live activity feed.
 *
 * Events arrive over the WebSocket as they happen and are shown immediately;
 * the API is read once, on arrival, so a dashboard opened mid-session starts
 * with history rather than an empty list.
 */

import * as React from "react";
import { Pause, Play } from "lucide-react";

import { api, type MetricEvent } from "../lib/api";
import type { ConnectionState } from "../lib/events";
import { compact, duration, exact, percent } from "../lib/format";
import { hintOf, useApi } from "../lib/useApi";
import { ActivityList, mergeEvents, outcomeVariant } from "../components/activity-feed";
import { PageBody, PageHeader } from "../components/page";
import { useScope } from "../components/scope";
import {
  Badge,
  Button,
  Card,
  CardContent,
  Failure,
  Notice,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  SkeletonRows,
} from "../components/ui";

/** How many events the feed holds. Beyond this, older ones fall off. */
const KEEP = 200;

const OUTCOMES = ["all", "success", "degraded", "failed"] as const;

export function Activity({
  live,
  connection,
  missed,
  revision,
}: {
  live: MetricEvent[];
  connection: ConnectionState;
  missed: number;
  revision: number;
}) {
  const { project, selected } = useScope();
  const [paused, setPaused] = React.useState(false);
  const [outcome, setOutcome] = React.useState<(typeof OUTCOMES)[number]>("all");
  const [operation, setOperation] = React.useState("all");

  // Read once per project change rather than on every event: the stream already
  // carries what is new, and re-reading on each frame would undo the point of
  // having a stream.
  const history = useApi(() => api.activity(KEEP, project), [project]);

  // Paused means "stop the list moving", not "stop listening". The stream keeps
  // running and the feed catches up when it is resumed.
  const frozen = React.useRef<MetricEvent[]>([]);
  const merged = mergeEvents(live, history.data, project, KEEP);
  if (!paused) frozen.current = merged;
  const events = paused ? frozen.current : merged;

  const operations = Array.from(
    new Set(events.map((event) => event.operation)),
  ).sort();

  const shown = events.filter(
    (event) =>
      (outcome === "all" || event.outcome === outcome) &&
      (operation === "all" || event.operation === operation),
  );

  const failures = events.filter((event) => event.outcome === "failed").length;
  const degraded = events.filter((event) => event.outcome === "degraded").length;

  return (
    <PageBody>
      <PageHeader
        title="Activity"
        description={
          selected
            ? `Operations on ${selected.name}, newest first.`
            : "Every operation the daemon has recorded, newest first."
        }
        actions={
          <>
            <Select
              value={operation}
              onValueChange={setOperation}
            >
              <SelectTrigger size="sm" className="w-36" aria-label="Operation">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All operations</SelectItem>
                {operations.map((name) => (
                  <SelectItem key={name} value={name}>
                    {name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select
              value={outcome}
              onValueChange={(value) =>
                setOutcome(value as (typeof OUTCOMES)[number])
              }
            >
              <SelectTrigger size="sm" className="w-32" aria-label="Outcome">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All outcomes</SelectItem>
                <SelectItem value="success">Succeeded</SelectItem>
                <SelectItem value="degraded">Degraded</SelectItem>
                <SelectItem value="failed">Failed</SelectItem>
              </SelectContent>
            </Select>

            <Button
              variant={paused ? "default" : "outline"}
              size="sm"
              onClick={() => setPaused((current) => !current)}
            >
              {paused ? <Play /> : <Pause />}
              {paused ? "Resume" : "Pause"}
            </Button>
          </>
        }
      >
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <Badge variant="outline">{exact(events.length)} held</Badge>
          {failures > 0 ? (
            <Badge variant="destructive">{exact(failures)} failed</Badge>
          ) : null}
          {degraded > 0 ? (
            <Badge variant="warning">{exact(degraded)} degraded</Badge>
          ) : null}
          {paused ? <Badge variant="info">Paused</Badge> : null}
        </div>
      </PageHeader>

      {history.error ? (
        <Failure
          message={history.error.message}
          hint={hintOf(history.error)}
          onRetry={history.reload}
        />
      ) : null}

      {missed > 0 ? (
        <Notice tone="warning" title={`${exact(missed)} events were missed`}>
          The dashboard fell behind the daemon and those events were dropped
          rather than queued. Totals on the other screens are read from the
          database and are unaffected.
        </Notice>
      ) : null}

      {connection !== "live" ? (
        <Notice tone="warning" title="Not receiving live events">
          This list stops growing until the connection comes back. Nothing is
          lost — the daemon records everything regardless.
        </Notice>
      ) : null}

      <Card>
        <CardContent className="px-5 py-2">
          {history.loading && events.length === 0 ? (
            <div className="py-3">
              <SkeletonRows rows={8} height="h-9" />
            </div>
          ) : (
            <ActivityList
              events={shown}
              emptyHint={
                events.length > 0
                  ? "Nothing matches these filters."
                  : undefined
              }
            />
          )}
        </CardContent>
      </Card>

      {shown.length > 0 ? <Failures events={shown} revision={revision} /> : null}
    </PageBody>
  );
}

/**
 * What went wrong, spelled out.
 *
 * A failed operation carries the daemon's own message. It is the one thing in
 * the feed that a row cannot show in full, and the one thing worth reading.
 */
function Failures({
  events,
  revision,
}: {
  events: MetricEvent[];
  revision: number;
}) {
  const troubled = events.filter(
    (event) => event.outcome !== "success" && event.detail,
  );
  if (troubled.length === 0) return null;

  return (
    <Card key={revision}>
      <CardContent className="space-y-3 px-5 py-4">
        <h2 className="text-sm font-semibold">What went wrong</h2>
        <ul className="space-y-2">
          {troubled.slice(0, 12).map((event, index) => (
            <li
              key={`${event.recorded_at}-${index}`}
              className="flex flex-wrap items-start gap-2 text-xs"
            >
              <Badge variant={outcomeVariant(event.outcome)}>
                {event.operation}
              </Badge>
              <span className="text-muted-foreground min-w-0 flex-1 break-words">
                {event.detail}
              </span>
              <span className="text-muted-foreground tabular shrink-0">
                {event.input_tokens > 0
                  ? `${compact(event.input_tokens)} tokens, `
                  : ""}
                {duration(event.duration_ms)}
              </span>
            </li>
          ))}
        </ul>
        <p className="text-muted-foreground text-xs">
          {percent(troubled.length / events.length)} of the operations shown did
          not fully succeed.
        </p>
      </CardContent>
    </Card>
  );
}
