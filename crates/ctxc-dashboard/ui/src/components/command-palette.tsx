/**
 * Everything, from the keyboard.
 *
 * Two kinds of thing live here. Places — the pages, and each registered project
 * — so navigation never needs the mouse. And actions that the dashboard can
 * genuinely perform: pausing a project, re-indexing one, switching theme.
 *
 * Nothing appears here that is not also a control on a page. A palette is a
 * faster way to reach the product, not a second product with its own features.
 */

import * as React from "react";
import {
  FolderGit2,
  Laptop,
  Moon,
  Pause,
  Play,
  RefreshCw,
  Sun,
} from "lucide-react";

import { api, type Project } from "../lib/api";
import { href, navigate } from "../lib/router";
import type { ThemeControl } from "../lib/theme";
import { NAVIGATION } from "./shell";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
  toast,
} from "./ui";

export function CommandPalette({
  open,
  onOpenChange,
  projects,
  theme,
  onChanged,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projects: Project[];
  theme: ThemeControl;
  /** Called after an action, so the pages behind re-read. */
  onChanged: () => void;
}) {
  const run = React.useCallback(
    (action: () => void) => {
      onOpenChange(false);
      action();
    },
    [onOpenChange],
  );

  const act = React.useCallback(
    async (project: Project, what: "pause" | "resume" | "reindex") => {
      onOpenChange(false);
      try {
        if (what === "pause") await api.pauseProject(project.id);
        if (what === "resume") await api.resumeProject(project.id);
        if (what === "reindex") await api.reindexProject(project.id);
        toast.success(
          what === "pause"
            ? `Paused ${project.name}.`
            : what === "resume"
              ? `Resumed ${project.name}.`
              : `Re-indexed ${project.name}.`,
        );
        onChanged();
      } catch (cause) {
        toast.error(cause instanceof Error ? cause.message : String(cause));
      }
    },
    [onChanged, onOpenChange],
  );

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder="Go to a page, or act on a project…" />
      <CommandList>
        <CommandEmpty>Nothing matches that.</CommandEmpty>

        {NAVIGATION.map((group) => (
          <CommandGroup key={group.label} heading={group.label}>
            {group.items.map((item) => (
              <CommandItem
                key={item.route}
                value={`${item.label} ${item.description}`}
                onSelect={() => run(() => navigate(href(item.route)))}
              >
                <item.icon />
                <span>{item.label}</span>
                <span className="text-muted-foreground ml-auto hidden truncate text-xs sm:block">
                  {item.description}
                </span>
              </CommandItem>
            ))}
          </CommandGroup>
        ))}

        {projects.length > 0 ? (
          <>
            <CommandSeparator />
            <CommandGroup heading="Projects">
              {projects.map((project) => (
                <CommandItem
                  key={project.id}
                  value={`project ${project.name} ${project.path}`}
                  onSelect={() =>
                    run(() => navigate(href("/projects", project.id)))
                  }
                >
                  <FolderGit2 />
                  <span className="truncate">{project.name}</span>
                  <span className="text-muted-foreground ml-auto hidden truncate font-mono text-xs md:block">
                    {project.path}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>

            <CommandGroup heading="Project actions">
              {projects.map((project) =>
                project.status === "paused" ? (
                  <CommandItem
                    key={`resume-${project.id}`}
                    value={`resume ${project.name}`}
                    onSelect={() => act(project, "resume")}
                  >
                    <Play />
                    Resume {project.name}
                  </CommandItem>
                ) : (
                  <CommandItem
                    key={`pause-${project.id}`}
                    value={`pause ${project.name}`}
                    onSelect={() => act(project, "pause")}
                  >
                    <Pause />
                    Pause {project.name}
                  </CommandItem>
                ),
              )}
              {projects.map((project) => (
                <CommandItem
                  key={`reindex-${project.id}`}
                  value={`re-index reindex ${project.name}`}
                  onSelect={() => act(project, "reindex")}
                >
                  <RefreshCw />
                  Re-index {project.name}
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        ) : null}

        <CommandSeparator />
        <CommandGroup heading="Appearance">
          <CommandItem value="light theme" onSelect={() => run(() => theme.set("light"))}>
            <Sun /> Light theme
          </CommandItem>
          <CommandItem value="dark theme" onSelect={() => run(() => theme.set("dark"))}>
            <Moon /> Dark theme
          </CommandItem>
          <CommandItem
            value="system theme"
            onSelect={() => run(() => theme.set("system"))}
          >
            <Laptop /> Follow the system
          </CommandItem>
        </CommandGroup>

        <CommandGroup heading="Data">
          <CommandItem value="refresh reload" onSelect={() => run(onChanged)}>
            <RefreshCw /> Re-read everything
            <CommandShortcut>R</CommandShortcut>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}

/** Open the palette on the shortcut every desktop application uses for it. */
export function usePaletteShortcut(open: (open: boolean) => void) {
  React.useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        open(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);
}
