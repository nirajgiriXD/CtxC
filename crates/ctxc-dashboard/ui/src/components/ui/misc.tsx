import * as React from "react";
import * as SeparatorPrimitive from "@radix-ui/react-separator";
import * as ScrollAreaPrimitive from "@radix-ui/react-scroll-area";

import { cn } from "../../lib/utils";

export function Separator({
  className,
  orientation = "horizontal",
  decorative = true,
  ...props
}: React.ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      decorative={decorative}
      orientation={orientation}
      className={cn(
        "bg-border shrink-0",
        orientation === "horizontal" ? "h-px w-full" : "h-full w-px",
        className,
      )}
      {...props}
    />
  );
}

export function ScrollArea({
  className,
  viewportClassName,
  children,
  ...props
}: React.ComponentProps<typeof ScrollAreaPrimitive.Root> & {
  viewportClassName?: string;
}) {
  return (
    <ScrollAreaPrimitive.Root
      className={cn("relative overflow-hidden", className)}
      {...props}
    >
      <ScrollAreaPrimitive.Viewport
        className={cn("size-full rounded-[inherit]", viewportClassName)}
      >
        {children}
      </ScrollAreaPrimitive.Viewport>
      <ScrollAreaPrimitive.Scrollbar
        orientation="vertical"
        className="flex w-2.5 touch-none p-0.5 transition-colors select-none"
      >
        <ScrollAreaPrimitive.Thumb className="bg-border hover:bg-muted-foreground relative flex-1 rounded-full transition-colors" />
      </ScrollAreaPrimitive.Scrollbar>
      <ScrollAreaPrimitive.Corner />
    </ScrollAreaPrimitive.Root>
  );
}

/**
 * A horizontal proportion bar.
 *
 * Used wherever one number has to be read against another — savings by stage,
 * how much of the index a language accounts for. The value can be negative:
 * a stage that costs tokens rather than saving them is a real outcome and must
 * not be flattened to zero.
 */
export function Meter({
  value,
  max,
  tone = "primary",
  className,
}: {
  value: number;
  max: number;
  tone?: "primary" | "warning" | "success" | "destructive" | "muted";
  className?: string;
}) {
  const width = max > 0 ? Math.min(100, (Math.abs(value) / max) * 100) : 0;
  const fill = {
    primary: "bg-primary",
    warning: "bg-warning",
    success: "bg-success",
    destructive: "bg-destructive",
    muted: "bg-muted-foreground",
  }[tone];

  return (
    <span
      className={cn(
        "bg-muted block h-2 w-full overflow-hidden rounded-full",
        className,
      )}
    >
      <span
        className={cn("block h-full rounded-full transition-[width]", fill)}
        style={{ width: `${width}%` }}
      />
    </span>
  );
}

/** A keyboard key, as printed on one. */
export function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "bg-muted text-muted-foreground inline-flex h-5 min-w-5 items-center justify-center",
        "rounded border px-1 font-sans text-[0.7rem] font-medium",
        className,
      )}
      {...props}
    />
  );
}
