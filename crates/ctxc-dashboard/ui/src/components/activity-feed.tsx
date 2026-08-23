/**
 * What CtxC has been doing, as a list.
 *
 * Everything here is untrusted. A `source` is a command line or a file path
 * from someone's repository, and a `detail` is an error message that may quote
 * file contents. React escapes it all, and nothing on this screen is ever
 * inserted as markup.
 */

import * as React from "react";

import type { MetricEvent } from "../lib/api";
import { clock, compact, duration, percent, stamp, title } from "../lib/format";
import { cn } from "../lib/utils";
import { Badge, Empty, Tooltip } from "./ui";
import { Activity as ActivityIcon } from "lucide-react";

/** How an outcome is coloured, everywhere it appears. */
export function outcomeVariant(outcome: MetricEvent["outcome"]) {
  return outcome === "failed"
    ? ("destructive" as const)
    : outcome === "degraded"
      ? ("warning" as const)
      : ("success" as const);
}

/** Merge the live stream with what was read from the API. */
export function mergeEvents(
  live: MetricEvent[],
  history: MetricEvent[] | null,
  project: string | undefined,
  keep: number,
): MetricEvent[] {
  const seen = new Set<string>();

  return [...live, ...(history ?? [])]
    .filter((event) => {
      if (project && event.project_id !== project) return false;
      // The same event can arrive both ways: over the socket, and in the
      // history read that started at the same moment.
      const key = `${event.recorded_at}:${event.operation}:${event.source}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => right.recorded_at - left.recorded_at)
    .slice(0, keep);
}

export function ActivityList({
  events,
  dense = false,
  emptyHint,
}: {
  events: MetricEvent[];
  /** Drop the columns a narrow card has no room for. */
  dense?: boolean;
  emptyHint?: React.ReactNode;
}) {
  if (events.length === 0) {
    return (
      <Empty
        icon={ActivityIcon}
        title="Nothing has happened yet."
        hint={
          emptyHint ??
          "Optimize something, search a project, or let the daemon index one."
        }
      />
    );
  }

  return (
    <ul className="divide-border/60 divide-y">
      {events.map((event, index) => (
        <ActivityRow
          key={`${event.recorded_at}-${event.operation}-${index}`}
          event={event}
          dense={dense}
        />
      ))}
    </ul>
  );
}

function ActivityRow({
  event,
  dense,
}: {
  event: MetricEvent;
  dense: boolean;
}) {
  const saved =
    event.input_tokens > 0
      ? (event.input_tokens - event.output_tokens) / event.input_tokens
      : 0;

  return (
    <li className="flex items-center gap-3 py-2.5 text-sm">
      <Tooltip label={stamp(event.recorded_at)}>
        <span className="text-muted-foreground tabular w-16 shrink-0 text-xs">
          {clock(event.recorded_at)}
        </span>
      </Tooltip>

      <Badge variant={outcomeVariant(event.outcome)} className="w-24 shrink-0 justify-center">
        {title(event.operation)}
      </Badge>

      <span className="min-w-0 flex-1 truncate font-mono text-xs" title={event.source}>
        {event.source}
      </span>

      {dense ? null : (
        <span className="text-muted-foreground tabular hidden w-28 shrink-0 text-right text-xs sm:block">
          {event.input_tokens > 0
            ? `${compact(event.input_tokens)} → ${compact(event.output_tokens)}`
            : "—"}
        </span>
      )}

      <span
        className={cn(
          "tabular w-14 shrink-0 text-right text-xs",
          saved > 0 ? "text-success" : "text-muted-foreground",
        )}
      >
        {saved > 0 ? percent(saved) : "—"}
      </span>

      <span className="text-muted-foreground tabular w-16 shrink-0 text-right text-xs">
        {duration(event.duration_ms)}
      </span>
    </li>
  );
}
