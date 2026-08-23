import * as React from "react";
import * as LabelPrimitive from "@radix-ui/react-label";

import { cn } from "../../lib/utils";

export function Label({
  className,
  ...props
}: React.ComponentProps<typeof LabelPrimitive.Root>) {
  return (
    <LabelPrimitive.Root
      data-slot="label"
      className={cn(
        "flex items-center gap-2 text-sm leading-none font-medium select-none " +
          "group-data-[disabled=true]:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export function Input({ className, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      data-slot="input"
      className={cn(
        "border-input bg-background flex h-9 w-full min-w-0 rounded-md border px-3 py-1 text-sm shadow-sm",
        "placeholder:text-muted-foreground/70 transition-[color,box-shadow,border-color] outline-none",
        "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/40",
        "aria-invalid:border-destructive aria-invalid:ring-destructive/20",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "file:text-foreground file:border-0 file:bg-transparent file:text-sm file:font-medium",
        className,
      )}
      {...props}
    />
  );
}

export function Textarea({
  className,
  ...props
}: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "border-input bg-background field-sizing-content min-h-20 w-full rounded-md border px-3 py-2 text-sm shadow-sm",
        "placeholder:text-muted-foreground/70 transition-[color,box-shadow,border-color] outline-none",
        "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/40",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

/**
 * One setting: a label, the control, and the sentence explaining it.
 *
 * The description sits under the control rather than under the label, because
 * it usually explains what a *value* means, and reading it after seeing the
 * current value is the order a person actually needs.
 */
export function Field({
  label,
  description,
  htmlFor,
  hint,
  error,
  className,
  children,
}: {
  label: React.ReactNode;
  description?: React.ReactNode;
  htmlFor?: string;
  /** Shown to the right of the label: where a value comes from, say. */
  hint?: React.ReactNode;
  error?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={htmlFor}>{label}</Label>
        {hint ? <span className="text-muted-foreground text-xs">{hint}</span> : null}
      </div>
      {children}
      {error ? (
        <p className="text-destructive text-xs">{error}</p>
      ) : description ? (
        <p className="text-muted-foreground text-xs leading-relaxed">
          {description}
        </p>
      ) : null}
    </div>
  );
}

/** A row of fields that should sit side by side when there is room. */
export function FieldRow({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("grid gap-5 sm:grid-cols-2", className)}
      {...props}
    />
  );
}
