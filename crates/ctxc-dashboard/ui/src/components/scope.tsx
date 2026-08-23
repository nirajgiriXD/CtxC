/**
 * What the dashboard is currently looking at.
 *
 * Two choices are shared by every screen: which project, and how far back. They
 * live above the pages rather than inside them so that narrowing to one project
 * on the overview keeps it narrowed when you open activity, and so that a live
 * event re-reading a panel never resets either one.
 *
 * Both are remembered for the session. Not for longer: a window scoped to one
 * project three weeks ago is a trap, whereas one scoped that way two minutes
 * ago is exactly what you left.
 */

import * as React from "react";

import type { Project } from "../lib/api";

/** The windows the metrics screens offer. */
export const WINDOWS = [
  { days: 1, label: "24h" },
  { days: 7, label: "7 days" },
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
] as const;

export interface Scope {
  /** Project id, or undefined for every project. */
  project?: string;
  setProject: (project?: string) => void;
  days: number;
  setDays: (days: number) => void;
  /** The projects the picker offers, as last read. */
  projects: Project[];
  /** The selected project, when it is one that still exists. */
  selected?: Project;
}

const ScopeContext = React.createContext<Scope | null>(null);

function remembered(key: string): string | undefined {
  return sessionStorage.getItem(key) ?? undefined;
}

export function ScopeProvider({
  projects,
  children,
}: {
  projects: Project[];
  children: React.ReactNode;
}) {
  const [project, setProjectState] = React.useState<string | undefined>(() =>
    remembered("ctxc-scope-project"),
  );
  const [days, setDaysState] = React.useState<number>(() => {
    const saved = Number(remembered("ctxc-scope-days"));
    return WINDOWS.some((window) => window.days === saved) ? saved : 7;
  });

  const setProject = React.useCallback((next?: string) => {
    if (next) sessionStorage.setItem("ctxc-scope-project", next);
    else sessionStorage.removeItem("ctxc-scope-project");
    setProjectState(next);
  }, []);

  const setDays = React.useCallback((next: number) => {
    sessionStorage.setItem("ctxc-scope-days", String(next));
    setDaysState(next);
  }, []);

  // A project that has been removed cannot stay selected: every scoped read
  // would fail with "not registered", which is a confusing way to learn that
  // something you were looking at is gone.
  const selected = projects.find((candidate) => candidate.id === project);
  React.useEffect(() => {
    if (project && projects.length > 0 && !selected) setProject(undefined);
  }, [project, projects.length, selected, setProject]);

  const value = React.useMemo(
    () => ({ project, setProject, days, setDays, projects, selected }),
    [project, setProject, days, setDays, projects, selected],
  );

  return <ScopeContext.Provider value={value}>{children}</ScopeContext.Provider>;
}

export function useScope(): Scope {
  const scope = React.useContext(ScopeContext);
  if (!scope) throw new Error("useScope must be used inside a ScopeProvider");
  return scope;
}
