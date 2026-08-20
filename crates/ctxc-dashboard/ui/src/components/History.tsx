/**
 * History charts.
 *
 * The daemon returns every bucket in the window, including the quiet ones, so a
 * gap on these charts means "nothing happened" rather than "no data arrived".
 * That distinction is the whole reason the timeseries endpoint zero-fills.
 */

import { useState } from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { api, type TimeseriesPoint } from "../api";
import { bucketLabel, compact, exact, percent } from "../format";
import { useApi } from "../useApi";
import { Button, Card, CardContent, CardHeader, CardTitle, Empty, Skeleton } from "./ui";

type Granularity = "hour" | "day";

export function History({
  days,
  project,
  revision,
}: {
  days: number;
  project?: string;
  revision: number;
}) {
  // Hours for a short window, days for a long one — 30 days of hourly buckets
  // is 720 points, which says less than 30 daily ones.
  const [granularity, setGranularity] = useState<Granularity>(
    days <= 2 ? "hour" : "day",
  );

  const series = useApi(
    () => api.timeseries(granularity, days, project),
    [granularity, days, project, revision],
  );

  const points = series.data?.points ?? [];
  const rows = points.map((point) => ({
    label: bucketLabel(point.bucket_start, granularity),
    ...point,
  }));

  const anything = points.some((point) => point.operations > 0);

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between">
        <CardTitle>History</CardTitle>
        <div className="flex gap-1">
          {(["hour", "day"] as const).map((option) => (
            <Button
              key={option}
              size="sm"
              variant={granularity === option ? "default" : "ghost"}
              onClick={() => setGranularity(option)}
            >
              {option === "hour" ? "Hourly" : "Daily"}
            </Button>
          ))}
        </div>
      </CardHeader>

      <CardContent>
        {series.loading && !series.data ? (
          <Skeleton className="h-64" />
        ) : !anything ? (
          <Empty title="Nothing recorded in this window yet." />
        ) : (
          <div className="grid gap-6 lg:grid-cols-2">
            <Panel title="Tokens before and after">
              <AreaChart data={rows}>
                {frame()}
                <Tooltip content={<Hint kind="tokens" />} />
                <Area
                  type="monotone"
                  dataKey="input_tokens"
                  name="Before"
                  stroke="var(--muted-foreground)"
                  fill="var(--muted-foreground)"
                  fillOpacity={0.15}
                  strokeWidth={1.5}
                />
                <Area
                  type="monotone"
                  dataKey="output_tokens"
                  name="After"
                  stroke="var(--primary)"
                  fill="var(--primary)"
                  fillOpacity={0.25}
                  strokeWidth={1.5}
                />
              </AreaChart>
            </Panel>

            <Panel title="Tokens saved">
              <BarChart data={rows}>
                {frame()}
                <Tooltip content={<Hint kind="saved" />} />
                <Bar dataKey="tokens_saved" name="Saved" fill="var(--primary)" radius={2} />
              </BarChart>
            </Panel>

            <Panel title="Operations">
              <BarChart data={rows}>
                {frame(exact)}
                <Tooltip content={<Hint kind="operations" />} />
                <Bar dataKey="operations" name="Operations" fill="var(--muted-foreground)" radius={2} />
              </BarChart>
            </Panel>

            <Panel title="Reduction">
              <LineChart data={rows}>
                {frame((value: number) => percent(value), [0, 1])}
                <Tooltip content={<Hint kind="reduction" />} />
                <Line
                  type="monotone"
                  dataKey="reduction_ratio"
                  name="Reduction"
                  stroke="var(--success)"
                  strokeWidth={1.5}
                  dot={false}
                />
              </LineChart>
            </Panel>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function Panel({ title, children }: { title: string; children: React.ReactElement }) {
  return (
    <div>
      <p className="text-muted-foreground mb-2 text-xs font-medium">{title}</p>
      <div className="h-48">
        <ResponsiveContainer width="100%" height="100%">
          {children}
        </ResponsiveContainer>
      </div>
    </div>
  );
}

/** The axes and grid every chart here shares. */
function frame(
  format: (value: number) => string = compact,
  domain?: [number, number],
) {
  return (
    <>
      <CartesianGrid stroke="var(--border)" strokeDasharray="3 3" vertical={false} />
      <XAxis
        dataKey="label"
        tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
        stroke="var(--border)"
        minTickGap={24}
      />
      <YAxis
        tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
        stroke="var(--border)"
        tickFormatter={format}
        width={48}
        domain={domain}
      />
    </>
  );
}

/**
 * A tooltip that reads as a sentence.
 *
 * Recharts' default shows raw field names, which are the API's vocabulary
 * rather than a person's.
 */
function Hint({
  active,
  payload,
  label,
  kind,
}: {
  active?: boolean;
  payload?: Array<{ payload: TimeseriesPoint }>;
  label?: string;
  kind: "tokens" | "saved" | "operations" | "reduction";
}) {
  const point = payload?.[0]?.payload;
  if (!active || !point) return null;

  const lines =
    kind === "tokens"
      ? [
          `${compact(point.input_tokens)} before`,
          `${compact(point.output_tokens)} after`,
        ]
      : kind === "saved"
        ? [`${compact(point.tokens_saved)} tokens saved`]
        : kind === "operations"
          ? [`${exact(point.operations)} operations`]
          : [`${percent(point.reduction_ratio)} smaller`];

  return (
    <div className="bg-card rounded-md border px-3 py-2 text-xs shadow-md">
      <p className="text-muted-foreground mb-1">{label}</p>
      {lines.map((line) => (
        <p key={line} className="tabular">
          {line}
        </p>
      ))}
      {point.errors > 0 ? (
        <p className="text-destructive mt-1">{exact(point.errors)} failed</p>
      ) : null}
    </div>
  );
}
