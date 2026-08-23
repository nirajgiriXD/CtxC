/**
 * The charts, and the one set of rules they all follow.
 *
 * Series colours come from the theme's `--chart-*` tokens rather than from
 * literals, so a chart is legible in both themes and one series keeps one
 * colour wherever it is drawn. Axes are quiet, gridlines are horizontal only,
 * and the tooltip is the same component everywhere — a reader should never have
 * to learn a second chart.
 */

import * as React from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
  XAxis,
  YAxis,
} from "recharts";

import { compact } from "../lib/format";
import { cn } from "../lib/utils";

/** The palette, in the order series should be assigned. */
export const SERIES = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
] as const;

const AXIS = {
  stroke: "var(--muted-foreground)",
  fontSize: 11,
  tickLine: false,
  axisLine: false,
} as const;

interface Point {
  label: string;
  [key: string]: string | number;
}

interface SeriesSpec {
  key: string;
  label: string;
  colour: string;
}

/** The shared tooltip: a heading, then one row per series. */
function ChartTooltip({
  active,
  payload,
  label,
  format,
}: {
  active?: boolean;
  payload?: { name?: string; dataKey?: string | number; value?: number; color?: string }[];
  label?: string;
  format: (value: number) => string;
}) {
  if (!active || !payload?.length) return null;

  return (
    <div className="bg-popover text-popover-foreground rounded-md border px-3 py-2 text-xs shadow-md">
      <p className="mb-1.5 font-medium">{label}</p>
      <div className="space-y-1">
        {payload.map((entry) => (
          <div
            key={String(entry.dataKey)}
            className="flex items-center justify-between gap-4"
          >
            <span className="flex items-center gap-1.5">
              <span
                className="size-2 rounded-full"
                style={{ backgroundColor: entry.color }}
              />
              <span className="text-muted-foreground">{entry.name}</span>
            </span>
            <span className="tabular font-medium">{format(entry.value ?? 0)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

/** A filled line over time. */
export function TrendChart({
  data,
  series,
  height = 240,
  format = compact,
  className,
}: {
  data: Point[];
  series: SeriesSpec[];
  height?: number;
  format?: (value: number) => string;
  className?: string;
}) {
  const gradients = React.useId();

  return (
    <div className={cn("w-full", className)} style={{ height }}>
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
          <defs>
            {series.map((entry, index) => (
              <linearGradient
                key={entry.key}
                id={`${gradients}-${index}`}
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop offset="0%" stopColor={entry.colour} stopOpacity={0.28} />
                <stop offset="100%" stopColor={entry.colour} stopOpacity={0.02} />
              </linearGradient>
            ))}
          </defs>
          <CartesianGrid
            vertical={false}
            stroke="var(--border)"
            strokeDasharray="3 3"
          />
          <XAxis dataKey="label" minTickGap={24} {...AXIS} />
          <YAxis width={52} tickFormatter={format} {...AXIS} />
          <RechartsTooltip
            cursor={{ stroke: "var(--border)" }}
            content={<ChartTooltip format={format} />}
          />
          {series.map((entry, index) => (
            <Area
              key={entry.key}
              type="monotone"
              dataKey={entry.key}
              name={entry.label}
              stroke={entry.colour}
              strokeWidth={2}
              fill={`url(#${gradients}-${index})`}
              dot={false}
              activeDot={{ r: 3, strokeWidth: 0 }}
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}

/** Counts per bucket. */
export function CountChart({
  data,
  series,
  height = 200,
  format = compact,
  className,
}: {
  data: Point[];
  series: SeriesSpec[];
  height?: number;
  format?: (value: number) => string;
  className?: string;
}) {
  return (
    <div className={cn("w-full", className)} style={{ height }}>
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
          <CartesianGrid
            vertical={false}
            stroke="var(--border)"
            strokeDasharray="3 3"
          />
          <XAxis dataKey="label" minTickGap={24} {...AXIS} />
          <YAxis width={44} tickFormatter={format} allowDecimals={false} {...AXIS} />
          <RechartsTooltip
            cursor={{ fill: "var(--muted)" }}
            content={<ChartTooltip format={format} />}
          />
          {series.map((entry) => (
            <Bar
              key={entry.key}
              dataKey={entry.key}
              name={entry.label}
              fill={entry.colour}
              radius={[3, 3, 0, 0]}
              maxBarSize={28}
            />
          ))}
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

/**
 * A single series with no axes, for a card that wants a shape rather than a
 * reading.
 */
export function Sparkline({
  data,
  dataKey,
  colour = SERIES[0],
  height = 44,
}: {
  data: Point[];
  dataKey: string;
  colour?: string;
  height?: number;
}) {
  const gradient = React.useId();

  return (
    <div className="w-full" style={{ height }}>
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ top: 2, right: 0, bottom: 0, left: 0 }}>
          <defs>
            <linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={colour} stopOpacity={0.3} />
              <stop offset="100%" stopColor={colour} stopOpacity={0} />
            </linearGradient>
          </defs>
          <Area
            type="monotone"
            dataKey={dataKey}
            stroke={colour}
            strokeWidth={1.5}
            fill={`url(#${gradient})`}
            dot={false}
            isAnimationActive={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}

/** A ranked bar chart, drawn sideways so the labels are readable. */
export function RankChart({
  data,
  height = 220,
  format = compact,
}: {
  data: { label: string; value: number }[];
  height?: number;
  format?: (value: number) => string;
}) {
  return (
    <div className="w-full" style={{ height }}>
      <ResponsiveContainer width="100%" height="100%">
        <BarChart
          data={data}
          layout="vertical"
          margin={{ top: 4, right: 16, bottom: 4, left: 4 }}
        >
          <CartesianGrid
            horizontal={false}
            stroke="var(--border)"
            strokeDasharray="3 3"
          />
          <XAxis type="number" tickFormatter={format} {...AXIS} />
          <YAxis
            type="category"
            dataKey="label"
            width={160}
            tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
            tickLine={false}
            axisLine={false}
          />
          <RechartsTooltip
            cursor={{ fill: "var(--muted)" }}
            content={<ChartTooltip format={format} />}
          />
          <Bar dataKey="value" name="Files" radius={[0, 3, 3, 0]} maxBarSize={18}>
            {data.map((entry, index) => (
              <Cell key={entry.label} fill={SERIES[index % SERIES.length]} />
            ))}
          </Bar>
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}
