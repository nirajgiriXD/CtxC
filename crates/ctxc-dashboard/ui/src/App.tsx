/**
 * The dashboard.
 *
 * A client of the daemon's HTTP API with no privileges of its own: everything
 * on screen came from a route the CLI can call too, and every control is one
 * call to a route `ctxc` can make as well. The WebSocket is only a notification
 * that something changed — each panel re-reads what it needs, so a dropped
 * frame costs a delay rather than a wrong number.
 */

import { useCallback, useState } from "react";

import { api, token } from "./lib/api";
import { useEventStream } from "./lib/events";
import { useLocation } from "./lib/router";
import { useTheme } from "./lib/theme";
import { useApi } from "./lib/useApi";
import { CommandPalette, usePaletteShortcut } from "./components/command-palette";
import { ScopeProvider } from "./components/scope";
import { Shell } from "./components/shell";
import { Code, Failure, Toaster, TooltipProvider } from "./components/ui";
import { Activity } from "./pages/activity";
import { Commands } from "./pages/commands";
import { Context } from "./pages/context";
import { Overview } from "./pages/overview";
import { Performance } from "./pages/performance";
import { Projects } from "./pages/projects";
import { Settings } from "./pages/settings";
import { System } from "./pages/system";

export function App() {
  const theme = useTheme();

  // Without a token nothing else can work, and every panel would show the same
  // 401. Say it once, at the top, with the command that fixes it.
  if (!token) {
    return (
      <div className="mx-auto flex min-h-screen max-w-lg items-center px-6">
        <Failure
          message="No access token."
          hint={
            <>
              open the dashboard with <Code>ctxc dashboard</Code>, which puts the
              token in the URL
            </>
          }
        />
      </div>
    );
  }

  return <Dashboard theme={theme} />;
}

function Dashboard({ theme }: { theme: ReturnType<typeof useTheme> }) {
  const location = useLocation();
  const stream = useEventStream();
  const [paletteOpen, setPaletteOpen] = useState(false);
  usePaletteShortcut(setPaletteOpen);

  const status = useApi(() => api.status(), [stream.revisions.watching, stream.revisions.projects]);
  const projects = useApi(() => api.projects(), [stream.revisions.projects]);

  const refreshing = status.refreshing || projects.refreshing;
  const refresh = useCallback(() => stream.refresh(), [stream]);

  return (
    <TooltipProvider>
      <ScopeProvider projects={projects.data ?? []}>
        <Shell
          location={location}
          status={status.data}
          connection={stream.state}
          onRefresh={refresh}
          refreshing={refreshing}
          onOpenPalette={() => setPaletteOpen(true)}
          theme={theme}
        >
          <Page
            route={location.route}
            detail={location.detail}
            revisions={stream.revisions}
            recent={stream.recent}
            connection={stream.state}
            missed={stream.missed}
            status={status}
          />
        </Shell>

        <CommandPalette
          open={paletteOpen}
          onOpenChange={setPaletteOpen}
          projects={projects.data ?? []}
          theme={theme}
          onChanged={refresh}
        />
      </ScopeProvider>
      <Toaster theme={theme.resolved} />
    </TooltipProvider>
  );
}

type PageProps = {
  route: ReturnType<typeof useLocation>["route"];
  detail?: string;
  revisions: ReturnType<typeof useEventStream>["revisions"];
  recent: ReturnType<typeof useEventStream>["recent"];
  connection: ReturnType<typeof useEventStream>["state"];
  missed: number;
  status: ReturnType<typeof useApi<Awaited<ReturnType<typeof api.status>>>>;
};

function Page({
  route,
  detail,
  revisions,
  recent,
  connection,
  missed,
  status,
}: PageProps) {
  switch (route) {
    case "/projects":
      return <Projects selected={detail} revisions={revisions} status={status.data} />;
    case "/activity":
      return (
        <Activity
          live={recent}
          connection={connection}
          missed={missed}
          revision={revisions.operations}
        />
      );
    case "/performance":
      return <Performance revision={revisions.operations} />;
    case "/context":
      return <Context revision={revisions.operations} />;
    case "/commands":
      return <Commands />;
    case "/settings":
      return <Settings revision={revisions.config} />;
    case "/system":
      return <System revision={revisions.any} status={status} />;
    case "/":
    default:
      return (
        <Overview
          revisions={revisions}
          status={status}
          recent={recent}
          connection={connection}
        />
      );
  }
}
