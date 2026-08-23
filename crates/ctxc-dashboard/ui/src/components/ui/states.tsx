/**
 * The states a panel is in when it has nothing to show.
 *
 * Loading, empty and broken are three different things and are never allowed to
 * look alike. An empty panel always says why it is empty and what would fill
 * it; a broken one always carries the daemon's own message and, where there is
 * one, the daemon's own hint — the same words `ctxc` would have printed.
 */

import * as React from "react";
import { AlertTriangle, Info, RefreshCw } from "lucide-react";

import { cn } from "../../lib/utils";
import { Button } from "./button";

export function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("bg-muted animate-pulse rounded-md", className)}
      {...props}
    />
  );
}

/** A stack of placeholder rows, for a list that has not arrived. */
export function SkeletonRows({
  rows = 4,
  height = "h-12",
}: {
  rows?: number;
  height?: string;
}) {
  return (
    <div className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, index) => (
        <Skeleton key={index} className={height} />
      ))}
    </div>
  );
}

export function Empty({
  icon: Icon,
  title,
  hint,
  action,
  className,
}: {
  icon?: React.ComponentType<{ className?: string }>;
  title: string;
  hint?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center gap-2 px-6 py-12 text-center",
        className,
      )}
    >
      {Icon ? (
        <span className="bg-muted text-muted-foreground mb-1 rounded-full p-2.5">
          <Icon className="size-5" />
        </span>
      ) : null}
      <p className="text-sm font-medium">{title}</p>
      {hint ? (
        <p className="text-muted-foreground max-w-md text-xs leading-relaxed">
          {hint}
        </p>
      ) : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}

/** A failure, showing the daemon's own message and hint. */
export function Failure({
  message,
  hint,
  onRetry,
  className,
}: {
  message: string;
  hint?: React.ReactNode;
  onRetry?: () => void;
  className?: string;
}) {
  return (
    <div
      role="alert"
      className={cn(
        "border-destructive/30 bg-destructive/5 flex flex-wrap items-start gap-3 rounded-lg border p-4",
        className,
      )}
    >
      <AlertTriangle className="text-destructive mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="text-destructive text-sm font-medium">{message}</p>
        {hint ? (
          <p className="text-muted-foreground text-xs leading-relaxed">
            Try: {hint}
          </p>
        ) : null}
      </div>
      {onRetry ? (
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RefreshCw /> Retry
        </Button>
      ) : null}
    </div>
  );
}

/** Something worth saying that is not a failure. */
export function Notice({
  tone = "info",
  title,
  children,
  action,
  className,
}: {
  tone?: "info" | "warning";
  title?: React.ReactNode;
  children?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}) {
  const styles =
    tone === "warning"
      ? "border-warning/30 bg-warning/5 [&_svg]:text-warning"
      : "border-info/30 bg-info/5 [&_svg]:text-info";

  return (
    <div
      className={cn(
        "flex flex-wrap items-start gap-3 rounded-lg border p-4",
        styles,
        className,
      )}
    >
      {tone === "warning" ? (
        <AlertTriangle className="mt-0.5 size-4 shrink-0" />
      ) : (
        <Info className="mt-0.5 size-4 shrink-0" />
      )}
      <div className="min-w-0 flex-1 space-y-1">
        {title ? <p className="text-sm font-medium">{title}</p> : null}
        {children ? (
          <div className="text-muted-foreground text-xs leading-relaxed">
            {children}
          </div>
        ) : null}
      </div>
      {action}
    </div>
  );
}

/** Monospaced text that is a command or a path, not prose. */
export function Code({ className, ...props }: React.ComponentProps<"code">) {
  return (
    <code
      className={cn(
        "bg-muted text-foreground rounded px-1.5 py-0.5 font-mono text-[0.8em]",
        className,
      )}
      {...props}
    />
  );
}
