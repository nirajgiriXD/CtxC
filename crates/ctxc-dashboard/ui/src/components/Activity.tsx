/**
 * The live activity feed.
 *
 * Events arrive over the WebSocket as they happen and are shown immediately;
 * the API is read once, on arrival, so that a dashboard opened mid-session
 * starts with history rather than an empty list.
 *
 * Everything here is untrusted. A `source` is a command line or a file path
 * from someone's repository, and a `detail` is an error message that may quote
 * file contents. React escapes it all, and nothing on this screen is ever
 * inserted as markup.
 */

import { api, type MetricEvent } from "../api";
import { clock, compact, duration, percent } from "../format";
import { useApi } from "../useApi";
import { Badge, Card, CardContent, CardHeader, CardTitle, Empty, Skeleton } from "./ui";

export function Activity({
  live,
  project,
  connection,
}: {
  live: MetricEvent[];
  project?: string;
  connection: string;
}) {
  // Read once per project change rather than on every event: the stream
  // already carries what is new, and re-reading on each frame would undo the
  // point of having a stream.
  const history = useApi(() => api.activity(50, project), [project]);

  const seen = new Set<string>();
  const events = [...live, ...(history.data ?? [])]
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
    .slice(0, 50);

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between">
        <CardTitle>Activity</CardTitle>
        <span className="text-muted-foreground text-xs">
          {connection === "live" ? "Live" : "Reconnecting…"}
        </span>
      </CardHeader>
      <CardContent>
        {history.loading && events.length === 0 ? (
          <div className="flex flex-col gap-2">
            {Array.from({ length: 6 }, (_, index) => (
              <Skeleton key={index} className="h-8" />
            ))}
          </div>
        ) : events.length === 0 ? (
          <Empty
            title="Nothing has happened yet."
            hint="Optimize something, or save a file in a watched project."
          />
        ) : (
          <ul className="flex flex-col">
            {events.map((event, index) => (
              <Row key={`${event.recorded_at}-${index}`} event={event} />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function Row({ event }: { event: MetricEvent }) {
  const saved = event.input_tokens - event.output_tokens;
  const reduction =
    event.input_tokens > 0 ? saved / event.input_tokens : 0;

  return (
    <li className="border-border/50 flex items-center gap-3 border-b py-2 text-sm last:border-0">
      <span className="text-muted-foreground tabular w-20 shrink-0 text-xs">
        {clock(event.recorded_at)}
      </span>

      <span className="w-24 shrink-0 text-xs font-medium">{event.operation}</span>

      <span className="text-muted-foreground min-w-0 flex-1 truncate font-mono text-xs" title={event.source}>
        {event.source}
      </span>

      {event.outcome === "failed" ? (
        <Badge variant="destructive" title={event.detail}>
          Failed
        </Badge>
      ) : event.outcome === "degraded" ? (
        <Badge variant="warning" title={event.detail}>
          Degraded
        </Badge>
      ) : null}

      {event.cache.lookups > 0 ? (
        <span className="text-muted-foreground tabular hidden text-xs sm:inline">
          {percent(event.cache.hits / event.cache.lookups)} cached
        </span>
      ) : null}

      {event.duration_ms > 0 ? (
        <span className="text-muted-foreground tabular hidden w-16 shrink-0 text-right text-xs md:inline">
          {duration(event.duration_ms)}
        </span>
      ) : null}

      <span
        className={[
          "tabular w-24 shrink-0 text-right text-xs",
          saved > 0 ? "text-primary" : "text-muted-foreground",
        ].join(" ")}
      >
        {event.input_tokens === 0
          ? "—"
          : `${compact(saved)} (${percent(reduction)})`}
      </span>
    </li>
  );
}
