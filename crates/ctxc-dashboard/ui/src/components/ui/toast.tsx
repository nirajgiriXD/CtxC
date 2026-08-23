/**
 * Confirmations that do not need a place on the page.
 *
 * A project paused, a setting saved, an index rebuilt: things worth confirming
 * once and then forgetting. Failures are *not* sent here — they belong next to
 * the control that failed, where the person can see what to do about them.
 */

import { Toaster as Sonner, toast } from "sonner";

export { toast };

export function Toaster({ theme }: { theme: "light" | "dark" }) {
  return (
    <Sonner
      position="bottom-right"
      // Told the theme the shell resolved, rather than deciding again: the
      // toaster asking the operating system directly would disagree with the
      // page behind it whenever someone has overridden the system preference.
      theme={theme}
      toastOptions={{
        classNames: {
          toast:
            "!bg-popover !text-popover-foreground !border !border-border !shadow-lg !rounded-lg",
          description: "!text-muted-foreground",
          actionButton: "!bg-primary !text-primary-foreground",
          cancelButton: "!bg-muted !text-muted-foreground",
        },
      }}
    />
  );
}
