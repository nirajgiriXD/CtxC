import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "../../lib/utils";

const badgeVariants = cva(
  "inline-flex w-fit shrink-0 items-center justify-center gap-1.5 rounded-md border " +
    "px-2 py-0.5 text-xs font-medium whitespace-nowrap " +
    "[&>svg]:pointer-events-none [&>svg]:size-3",
  {
    variants: {
      variant: {
        default: "border-transparent bg-secondary text-secondary-foreground",
        outline: "text-muted-foreground",
        // Status colours are carried by text and a tinted border rather than a
        // solid fill: a table of twenty rows should not look like a paint chart.
        success: "border-success/30 bg-success/10 text-success",
        warning: "border-warning/30 bg-warning/10 text-warning",
        destructive:
          "border-destructive/30 bg-destructive/10 text-destructive",
        info: "border-info/30 bg-info/10 text-info",
        primary: "border-primary/30 bg-primary/10 text-primary",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

export function Badge({
  className,
  variant,
  asChild = false,
  ...props
}: React.ComponentProps<"span"> &
  VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Component = asChild ? Slot : "span";
  return (
    <Component
      data-slot="badge"
      className={cn(badgeVariants({ variant }), className)}
      {...props}
    />
  );
}

/** A small coloured disc, for a status that needs no words beside a label. */
export function Dot({
  tone = "muted",
  className,
  ...props
}: React.ComponentProps<"span"> & {
  tone?: "success" | "warning" | "destructive" | "info" | "muted";
}) {
  const colour = {
    success: "bg-success",
    warning: "bg-warning",
    destructive: "bg-destructive",
    info: "bg-info",
    muted: "bg-muted-foreground",
  }[tone];

  return (
    <span
      className={cn("inline-block size-2 shrink-0 rounded-full", colour, className)}
      {...props}
    />
  );
}
