/**
 * What CtxC has saved, and where the saving came from.
 *
 * The headline is deliberately not a single percentage. "62% smaller" is
 * trivia; a stage breakdown is something a person can act on, so it sits next
 * to the totals rather than three clicks away.
 */

import { api, type StageSavings, type Summary } from "../api";
import { compact, cost, duration, exact, percent } from "../format";
import { useApi } from "../useApi";
import { Card, CardContent, CardHeader, CardTitle, Empty, Failure, Skeleton } from "./ui";
import { History } from "./History";

export function Overview({
  days,
  project,
  revision,
}: {
  days: number;
  project?: string;
  revision: number;
}) {
  const summary = useApi(() => api.summary(days, project), [days, project, revision]);
  const breakdown = useApi(
    () => api.breakdown(days, project),
    [days, project, revision],
  );

  if (summary.error) {
    return (
      <Failure
        message={summary.error.message}
        hint={"hint" in summary.error ? summary.error.hint : undefined}
        onRetry={summary.reload}
      />
    );
  }

  if (summary.loading || !summary.data) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {Array.from({ length: 8 }, (_, index) => (
          <Skeleton key={index} className="h-24" />
        ))}
      </div>
    );
  }

  const data = summary.data;

  if (data.operations === 0) {
    return (
      <Card>
        <Empty
          title="Nothing recorded in this window yet."
          hint="Optimize something, or let the daemon index a project."
        />
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat label="Operations" value={exact(data.operations)} />
        <Stat
          label="Tokens saved"
          value={compact(data.tokens_saved)}
          detail={`${compact(data.input_tokens)} in, ${compact(data.output_tokens)} out`}
          emphasis
        />
        <Stat label="Reduction" value={percent(data.reduction_ratio)} />
        <CostStat summary={data} />
        <Stat
          label="Latency"
          value={
            data.average_duration_ms === undefined
              ? "—"
              : duration(data.average_duration_ms)
          }
          detail={
            data.slowest_duration_ms > 0
              ? `${duration(data.slowest_duration_ms)} slowest`
              : undefined
          }
        />
        <Stat
          label="Cache hit rate"
          value={
            data.cache_hit_rate === undefined ? "—" : percent(data.cache_hit_rate)
          }
          detail={
            data.cache_hit_rate === undefined ? "nothing used a cache" : undefined
          }
        />
        <Stat
          label="Errors"
          value={exact(data.errors)}
          detail={data.degradations > 0 ? `${data.degradations} degraded` : undefined}
          tone={data.errors > 0 ? "bad" : undefined}
        />
        <Stat
          label="Token counts"
          value={data.estimated ? "Estimated" : "Exact"}
          detail={data.estimated ? "not the target tokenizer" : undefined}
        />
      </div>

      <Stages savings={data.savings_by_stage} total={data.tokens_saved} />

      <History days={days} project={project} revision={revision} />

      <Card>
        <CardHeader>
          <CardTitle>By operation</CardTitle>
        </CardHeader>
        <CardContent>
          {breakdown.data && breakdown.data.by_operation.length > 0 ? (
            <table className="w-full text-sm">
              <thead className="text-muted-foreground text-xs">
                <tr className="border-b">
                  <th className="py-2 text-left font-medium">Operation</th>
                  <th className="py-2 text-right font-medium">Runs</th>
                  <th className="py-2 text-right font-medium">Saved</th>
                  <th className="py-2 text-right font-medium">Reduction</th>
                  <th className="py-2 text-right font-medium">Average</th>
                </tr>
              </thead>
              <tbody className="tabular">
                {breakdown.data.by_operation.map((row) => (
                  <tr key={row.operation} className="border-border/50 border-b last:border-0">
                    <td className="py-2">{row.operation}</td>
                    <td className="py-2 text-right">{exact(row.operations)}</td>
                    <td className="py-2 text-right">{compact(row.tokens_saved)}</td>
                    <td className="py-2 text-right">{percent(row.reduction_ratio)}</td>
                    <td className="text-muted-foreground py-2 text-right">
                      {row.average_duration_ms === undefined
                        ? "—"
                        : duration(row.average_duration_ms)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <Empty title="No operations in this window." />
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Stat({
  label,
  value,
  detail,
  emphasis,
  tone,
}: {
  label: string;
  value: string;
  detail?: string;
  emphasis?: boolean;
  tone?: "bad";
}) {
  return (
    <Card>
      <CardContent className="p-5">
        <p className="text-muted-foreground text-xs font-medium">{label}</p>
        <p
          className={[
            "tabular mt-1 text-2xl font-semibold",
            emphasis ? "text-primary" : "",
            tone === "bad" ? "text-destructive" : "",
          ].join(" ")}
        >
          {value}
        </p>
        {detail ? (
          <p className="text-muted-foreground mt-1 text-xs">{detail}</p>
        ) : null}
      </CardContent>
    </Card>
  );
}

/**
 * Cost, or a clear statement that nobody configured a rate.
 *
 * An unconfigured rate must not become "$0.00": that reads as "this saved
 * nothing" rather than "CtxC has no idea what your tokens cost".
 */
function CostStat({ summary }: { summary: Summary }) {
  const amount = cost(summary.estimated_cost_saved);

  return (
    <Stat
      label="Estimated cost saved"
      value={amount ?? "Not estimated"}
      detail={
        amount
          ? `estimate, ${summary.estimated_cost_saved?.model} rates`
          : "set metrics.cost_per_million_input_tokens"
      }
    />
  );
}

/** Where the reduction actually came from. */
function Stages({ savings, total }: { savings: StageSavings; total: number }) {
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
        <CardTitle>Savings by stage</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-center gap-3 text-sm">
            <span className="text-muted-foreground w-40 shrink-0 text-xs">
              {label}
            </span>
            <span className="bg-muted h-2 flex-1 overflow-hidden rounded-full">
              <span
                className={[
                  "block h-full rounded-full",
                  // A stage can cost tokens rather than save them; collapsing
                  // repeated log lines and noting how many adds a few back.
                  value < 0 ? "bg-warning" : "bg-primary",
                ].join(" ")}
                style={{ width: `${(Math.abs(value) / widest) * 100}%` }}
              />
            </span>
            <span className="tabular w-24 shrink-0 text-right text-xs">
              {value < 0 ? `+${compact(-value)} added` : compact(value)}
            </span>
          </div>
        ))}
        <p className="text-muted-foreground mt-1 text-xs">
          Adds up to {compact(total)} tokens saved.
        </p>
      </CardContent>
    </Card>
  );
}
