/**
 * The pieces every page is built from.
 *
 * Spacing, heading sizes and the shape of a statistic are decided once here, so
 * eight screens look like one product rather than eight.
 */

import * as React from "react";

import { cn } from "../lib/utils";
import { Card, CardContent, Tooltip } from "./ui";

/** The title block at the top of a page. */
export function PageHeader({
  title,
  description,
  actions,
  children,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0 space-y-1">
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        {description ? (
          <p className="text-muted-foreground max-w-2xl text-sm leading-relaxed">
            {description}
          </p>
        ) : null}
        {children}
      </div>
      {actions ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>
      ) : null}
    </header>
  );
}

/** A titled group of related panels. */
export function Section({
  title,
  description,
  actions,
  className,
  children,
}: {
  title?: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <section className={cn("space-y-3", className)}>
      {title || actions ? (
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div className="space-y-1">
            {title ? (
              <h2 className="text-sm font-semibold tracking-tight">{title}</h2>
            ) : null}
            {description ? (
              <p className="text-muted-foreground text-xs">{description}</p>
            ) : null}
          </div>
          {actions}
        </div>
      ) : null}
      {children}
    </section>
  );
}

/** The page's own vertical rhythm. */
export function PageBody({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div className={cn("animate-rise flex flex-col gap-6", className)} {...props} />
  );
}

/**
 * One number, with what it means underneath.
 *
 * `detail` carries the qualifier that keeps the headline honest — what the
 * average was taken over, or that a figure is an estimate — rather than leaving
 * a bare number to be read as more precise than it is.
 */
export function Stat({
  label,
  value,
  detail,
  icon: Icon,
  tone,
  help,
  className,
}: {
  label: React.ReactNode;
  value: React.ReactNode;
  detail?: React.ReactNode;
  icon?: React.ComponentType<{ className?: string }>;
  tone?: "primary" | "bad" | "good";
  help?: React.ReactNode;
  className?: string;
}) {
  const body = (
    <Card className={cn("overflow-hidden", className)}>
      <CardContent className="flex items-start gap-3 px-5 py-4">
        <div className="min-w-0 flex-1">
          <p className="text-muted-foreground text-xs font-medium">{label}</p>
          <p
            className={cn(
              "tabular mt-1.5 truncate text-2xl font-semibold tracking-tight",
              tone === "primary" && "text-primary",
              tone === "bad" && "text-destructive",
              tone === "good" && "text-success",
            )}
          >
            {value}
          </p>
          {detail ? (
            // Wrapped rather than truncated: the qualifier is what keeps the
            // headline honest, and half of "set metrics.cost_per_million…"
            // helps nobody.
            <p className="text-muted-foreground mt-1 text-xs leading-snug">
              {detail}
            </p>
          ) : null}
        </div>
        {Icon ? (
          <span className="bg-muted text-muted-foreground rounded-md p-2">
            <Icon className="size-4" />
          </span>
        ) : null}
      </CardContent>
    </Card>
  );

  return help ? (
    <Tooltip label={help} asChild={false}>
      {body}
    </Tooltip>
  ) : (
    body
  );
}

/** The grid statistics sit in. */
export function StatGrid({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("grid gap-4 sm:grid-cols-2 xl:grid-cols-4", className)}
      {...props}
    />
  );
}

/** A label and a value on one line, for detail panels. */
export function Detail({
  label,
  children,
  mono,
}: {
  label: React.ReactNode;
  children: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1 py-2">
      <dt className="text-muted-foreground text-xs">{label}</dt>
      <dd
        className={cn(
          "min-w-0 text-right text-sm break-all",
          mono && "font-mono text-xs",
        )}
      >
        {children}
      </dd>
    </div>
  );
}

export function DetailList({ className, ...props }: React.ComponentProps<"dl">) {
  return <dl className={cn("divide-border/60 divide-y", className)} {...props} />;
}
