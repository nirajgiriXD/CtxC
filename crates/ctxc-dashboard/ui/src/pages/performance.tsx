/**
 * Savings over time, by stage and by operation.
 *
 * The same numbers `ctxc status --metrics --breakdown --by day` prints, from the same
 * routes. Charts here are for shape and comparison; the tables underneath carry
 * the figures, because a chart is a bad place to read an exact number from.
 */

import { BarChart3 } from "lucide-react";

import { api } from "../lib/api";
import { bucketLabel, compact, cost, duration, exact, percent } from "../lib/format";
import { hintOf, useApi } from "../lib/useApi";
import { CountChart, SERIES, TrendChart } from "../components/charts";
import {
  PageBody,
  PageHeader,
  Section,
  Stat,
  StatGrid,
} from "../components/page";
import { useScope, WINDOWS } from "../components/scope";
import { Stages } from "./overview";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardHeading,
  CardTitle,
  Code,
  Empty,
  Failure,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "../components/ui";

export function Performance({ revision }: { revision: number }) {
  const { days, project, selected } = useScope();
  const granularity = days <= 1 ? "hour" : "day";
  const window = WINDOWS.find((option) => option.days === days);

  const summary = useApi(() => api.summary(days, project), [days, project, revision]);
  const breakdown = useApi(
    () => api.breakdown(days, project),
    [days, project, revision],
  );
  const timeseries = useApi(
    () => api.timeseries(granularity, days, project),
    [granularity, days, project, revision],
  );

  const points = (timeseries.data?.points ?? []).map((point) => ({
    label: bucketLabel(point.bucket_start, granularity),
    saved: point.tokens_saved,
    input: point.input_tokens,
    output: point.output_tokens,
    operations: point.operations,
    errors: point.errors,
  }));

  const anything = summary.data && summary.data.operations > 0;

  return (
    <PageBody>
      <PageHeader
        title="Performance"
        description={
          selected
            ? `${selected.name}, ${window?.label ?? `${days} days`}, one point per ${granularity}.`
            : `Every project, ${window?.label ?? `${days} days`}, one point per ${granularity}.`
        }
      />

      {summary.error ? (
        <Failure
          message={summary.error.message}
          hint={hintOf(summary.error)}
          onRetry={summary.reload}
        />
      ) : null}

      {summary.loading && !summary.data ? (
        <StatGrid>
          {Array.from({ length: 4 }, (_, index) => (
            <Skeleton key={index} className="h-[6.5rem]" />
          ))}
        </StatGrid>
      ) : !anything ? (
        <Card>
          <Empty
            icon={BarChart3}
            title="Nothing to measure in this window."
            hint={
              <>
                Metrics come from operations that actually ran. Widen the window,
                or run something — <Code>ctxc project index .</Code> is enough.
              </>
            }
          />
        </Card>
      ) : summary.data ? (
        <>
          <StatGrid>
            <Stat
              label="Tokens saved"
              value={compact(summary.data.tokens_saved)}
              detail={percent(summary.data.reduction_ratio) + " smaller"}
              tone="primary"
            />
            <Stat
              label="Operations"
              value={exact(summary.data.operations)}
              detail={`${exact(summary.data.errors)} failed`}
              tone={summary.data.errors > 0 ? "bad" : undefined}
            />
            <Stat
              label="Average duration"
              value={
                summary.data.average_duration_ms === undefined
                  ? "—"
                  : duration(summary.data.average_duration_ms)
              }
              detail={
                summary.data.slowest_duration_ms > 0
                  ? `${duration(summary.data.slowest_duration_ms)} slowest`
                  : undefined
              }
            />
            <Stat
              label="Estimated cost saved"
              value={cost(summary.data.estimated_cost_saved) ?? "Not estimated"}
              detail={
                summary.data.estimated_cost_saved
                  ? `at ${summary.data.estimated_cost_saved.model} rates`
                  : "no token rate is configured"
              }
            />
          </StatGrid>

          <Card>
            <CardHeader>
              <CardHeading>
                <CardTitle>Tokens in, out and saved</CardTitle>
                <CardDescription>
                  Saved is the difference. All three are estimates unless the
                  target tokenizer was available.
                </CardDescription>
              </CardHeading>
            </CardHeader>
            <CardContent>
              {timeseries.error ? (
                <Failure
                  message={timeseries.error.message}
                  hint={hintOf(timeseries.error)}
                  onRetry={timeseries.reload}
                />
              ) : points.length === 0 ? (
                <Empty title="No buckets in this window." />
              ) : (
                <TrendChart
                  data={points}
                  series={[
                    { key: "input", label: "Tokens in", colour: SERIES[2] },
                    { key: "output", label: "Tokens out", colour: SERIES[1] },
                    { key: "saved", label: "Saved", colour: SERIES[0] },
                  ]}
                />
              )}
            </CardContent>
          </Card>

          <div className="grid gap-6 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardHeading>
                  <CardTitle>Operations per {granularity}</CardTitle>
                  <CardDescription>
                    How busy CtxC has been, and where failures cluster.
                  </CardDescription>
                </CardHeading>
              </CardHeader>
              <CardContent>
                {points.length === 0 ? (
                  <Empty title="No buckets in this window." />
                ) : (
                  <CountChart
                    data={points}
                    series={[
                      { key: "operations", label: "Operations", colour: SERIES[0] },
                      { key: "errors", label: "Errors", colour: "var(--destructive)" },
                    ]}
                    format={(value) => exact(value)}
                  />
                )}
              </CardContent>
            </Card>

            <Stages
              savings={summary.data.savings_by_stage}
              total={summary.data.tokens_saved}
            />
          </div>

          <Section
            title="By operation"
            description="Every kind of work CtxC did in this window."
          >
            <Card>
              <CardContent className="px-0 pb-0">
                {breakdown.error ? (
                  <div className="p-5">
                    <Failure
                      message={breakdown.error.message}
                      hint={hintOf(breakdown.error)}
                      onRetry={breakdown.reload}
                    />
                  </div>
                ) : breakdown.data && breakdown.data.by_operation.length > 0 ? (
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead className="pl-5">Operation</TableHead>
                        <TableHead numeric>Runs</TableHead>
                        <TableHead numeric>In</TableHead>
                        <TableHead numeric>Out</TableHead>
                        <TableHead numeric>Saved</TableHead>
                        <TableHead numeric>Reduction</TableHead>
                        <TableHead numeric>Average</TableHead>
                        <TableHead numeric className="pr-5">
                          Errors
                        </TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {breakdown.data.by_operation.map((row) => (
                        <TableRow key={row.operation}>
                          <TableCell className="pl-5 font-medium">
                            {row.operation}
                          </TableCell>
                          <TableCell numeric>{exact(row.operations)}</TableCell>
                          <TableCell numeric className="text-muted-foreground">
                            {compact(row.input_tokens)}
                          </TableCell>
                          <TableCell numeric className="text-muted-foreground">
                            {compact(row.output_tokens)}
                          </TableCell>
                          <TableCell numeric>{compact(row.tokens_saved)}</TableCell>
                          <TableCell numeric>
                            {percent(row.reduction_ratio)}
                          </TableCell>
                          <TableCell numeric className="text-muted-foreground">
                            {row.average_duration_ms === undefined
                              ? "—"
                              : duration(row.average_duration_ms)}
                          </TableCell>
                          <TableCell
                            numeric
                            className={
                              row.errors > 0 ? "text-destructive pr-5" : "pr-5"
                            }
                          >
                            {exact(row.errors)}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                ) : (
                  <Empty title="No operations in this window." />
                )}
              </CardContent>
            </Card>
          </Section>

          {summary.data.estimated ? (
            <p className="text-muted-foreground text-xs">
              Token counts on this page are estimates: CtxC counted them with its
              own heuristic rather than the tokenizer of the model you are using.
              Costs derived from them inherit that.
            </p>
          ) : null}
        </>
      ) : null}
    </PageBody>
  );
}
