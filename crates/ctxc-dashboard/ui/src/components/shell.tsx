/**
 * The frame every page sits in.
 *
 * A sidebar naming the areas of the product, and a top bar carrying the things
 * that are true of whatever page is open: what the dashboard is scoped to,
 * whether the daemon is still talking to us, and the way out to settings and
 * the command menu.
 *
 * The sidebar collapses to icons on a narrow desktop and folds into a drawer on
 * a phone. Which it is doing is remembered, because a person who collapsed it
 * did so on purpose.
 */

import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import {
  Activity as ActivityIcon,
  ChevronsLeft,
  ChevronsRight,
  CircleSlash,
  FolderGit2,
  Gauge,
  Laptop,
  Menu,
  Monitor,
  Moon,
  PanelsTopLeft,
  Power,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  Sun,
  Terminal,
  TrendingUp,
} from "lucide-react";

import { api, type DaemonStatus } from "../lib/api";
import type { ConnectionState } from "../lib/events";
import { href, Link, type Location, type Route } from "../lib/router";
import type { Theme, ThemeControl } from "../lib/theme";
import { cn } from "../lib/utils";
import { useScope, WINDOWS } from "./scope";
import {
  Badge,
  Button,
  Code,
  Dot,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Kbd,
  Confirm,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  toast,
  Tooltip,
} from "./ui";

interface NavItem {
  route: Route;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  description: string;
}

interface NavGroup {
  label: string;
  items: NavItem[];
}

/** The map of the product. Every entry is a page that exists. */
export const NAVIGATION: NavGroup[] = [
  {
    label: "Monitor",
    items: [
      {
        route: "/",
        label: "Overview",
        icon: Gauge,
        description: "What CtxC has saved, and what it is doing now",
      },
      {
        route: "/projects",
        label: "Projects",
        icon: FolderGit2,
        description: "Register, pause, re-index and inspect projects",
      },
      {
        route: "/activity",
        label: "Activity",
        icon: ActivityIcon,
        description: "Every operation as it happens",
      },
      {
        route: "/performance",
        label: "Performance",
        icon: TrendingUp,
        description: "Savings over time, by stage and by operation",
      },
    ],
  },
  {
    label: "Work",
    items: [
      {
        route: "/context",
        label: "Context",
        icon: Search,
        description: "Search the index and read what CtxC selected",
      },
      {
        route: "/commands",
        label: "Commands",
        icon: Terminal,
        description: "Everything the CLI can do, and where it lives here",
      },
    ],
  },
  {
    label: "Manage",
    items: [
      {
        route: "/settings",
        label: "Settings",
        icon: SettingsIcon,
        description: "The configuration file, edited safely",
      },
      {
        route: "/system",
        label: "System",
        icon: Monitor,
        description: "Daemon, storage, logs and diagnostics",
      },
    ],
  },
];

/** Find the entry for a route, for titles and breadcrumbs. */
export function navItem(route: Route): NavItem {
  const found = NAVIGATION.flatMap((group) => group.items).find(
    (item) => item.route === route,
  );
  // Every route in the router has an entry here, and the test that keeps them
  // in step is the router's own `ROUTES` list.
  return found ?? NAVIGATION[0]!.items[0]!;
}

export function Shell({
  location,
  status,
  connection,
  onRefresh,
  refreshing,
  onOpenPalette,
  theme,
  children,
}: {
  location: Location;
  status: DaemonStatus | null;
  connection: ConnectionState;
  onRefresh: () => void;
  refreshing: boolean;
  onOpenPalette: () => void;
  theme: ThemeControl;
  children: React.ReactNode;
}) {
  const [collapsed, setCollapsed] = React.useState(
    () => localStorage.getItem("ctxc-sidebar") === "collapsed",
  );
  const [drawerOpen, setDrawerOpen] = React.useState(false);

  const toggle = () => {
    setCollapsed((current) => {
      localStorage.setItem("ctxc-sidebar", current ? "open" : "collapsed");
      return !current;
    });
  };

  // Following a link on a phone should put the page back, not leave the drawer
  // covering what was just navigated to.
  React.useEffect(() => setDrawerOpen(false), [location.path]);

  return (
    <div className="flex min-h-screen">
      <aside
        className={cn(
          "bg-sidebar border-sidebar-border sticky top-0 hidden h-screen shrink-0 border-r lg:flex lg:flex-col",
          "transition-[width] duration-200",
          collapsed ? "w-[4.25rem]" : "w-60",
        )}
      >
        <Brand collapsed={collapsed} version={status?.version} />
        <Nav current={location.route} collapsed={collapsed} />
        <div className="border-sidebar-border mt-auto border-t p-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={toggle}
            className="text-muted-foreground w-full justify-start"
            aria-label={collapsed ? "Expand the sidebar" : "Collapse the sidebar"}
          >
            {collapsed ? <ChevronsRight /> : <ChevronsLeft />}
            {collapsed ? null : <span>Collapse</span>}
          </Button>
        </div>
      </aside>

      <MobileNav
        open={drawerOpen}
        onOpenChange={setDrawerOpen}
        current={location.route}
        version={status?.version}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar
          location={location}
          status={status}
          connection={connection}
          onRefresh={onRefresh}
          refreshing={refreshing}
          onOpenPalette={onOpenPalette}
          onOpenDrawer={() => setDrawerOpen(true)}
          theme={theme}
        />
        <main className="mx-auto w-full max-w-[1400px] flex-1 px-4 py-6 sm:px-6 lg:px-8">
          {children}
        </main>
      </div>
    </div>
  );
}

function Brand({
  collapsed,
  version,
}: {
  collapsed: boolean;
  version?: string;
}) {
  return (
    <div
      className={cn(
        "border-sidebar-border flex h-14 items-center gap-2.5 border-b px-4",
        collapsed && "justify-center px-0",
      )}
    >
      <span className="bg-primary text-primary-foreground grid size-7 shrink-0 place-items-center rounded-md">
        <PanelsTopLeft className="size-4" />
      </span>
      {collapsed ? null : (
        <span className="min-w-0">
          <span className="block text-sm leading-none font-semibold">CtxC</span>
          <span className="text-muted-foreground block text-[0.7rem] leading-tight">
            {version ? `v${version}` : "context compiler"}
          </span>
        </span>
      )}
    </div>
  );
}

function Nav({
  current,
  collapsed,
  onNavigate,
}: {
  current: Route;
  collapsed?: boolean;
  onNavigate?: () => void;
}) {
  return (
    <nav className="flex-1 space-y-4 overflow-y-auto p-2" aria-label="Sections">
      {NAVIGATION.map((group) => (
        <div key={group.label} className="space-y-1">
          {collapsed ? (
            <div className="bg-sidebar-border mx-3 my-2 h-px" />
          ) : (
            <p className="text-muted-foreground px-3 pt-1 text-[0.7rem] font-medium tracking-wide uppercase">
              {group.label}
            </p>
          )}
          {group.items.map((item) => {
            const active = item.route === current;
            const link = (
              <Link
                key={item.route}
                to={href(item.route)}
                onClick={onNavigate}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "flex items-center gap-2.5 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                  collapsed && "justify-center px-0",
                  active
                    ? "bg-sidebar-accent text-foreground"
                    : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground",
                )}
              >
                <item.icon className="size-4 shrink-0" />
                {collapsed ? (
                  <span className="sr-only">{item.label}</span>
                ) : (
                  <span className="truncate">{item.label}</span>
                )}
              </Link>
            );

            return collapsed ? (
              <Tooltip key={item.route} label={item.label} side="right">
                {link}
              </Tooltip>
            ) : (
              link
            );
          })}
        </div>
      ))}
    </nav>
  );
}

function MobileNav({
  open,
  onOpenChange,
  current,
  version,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  current: Route;
  version?: string;
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fade fixed inset-0 z-50 bg-black/50 lg:hidden" />
        <DialogPrimitive.Content
          className="bg-sidebar pop fixed inset-y-0 left-0 z-50 flex w-64 flex-col border-r shadow-xl lg:hidden"
          aria-label="Sections"
        >
          <DialogPrimitive.Title className="sr-only">Sections</DialogPrimitive.Title>
          <Brand collapsed={false} version={version} />
          <Nav current={current} onNavigate={() => onOpenChange(false)} />
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

/** Which pages the scope controls actually mean something on. */
const SCOPED: Route[] = ["/", "/projects", "/activity", "/performance", "/context"];
const WINDOWED: Route[] = ["/", "/performance"];

function TopBar({
  location,
  status,
  connection,
  onRefresh,
  refreshing,
  onOpenPalette,
  onOpenDrawer,
  theme,
}: {
  location: Location;
  status: DaemonStatus | null;
  connection: ConnectionState;
  onRefresh: () => void;
  refreshing: boolean;
  onOpenPalette: () => void;
  onOpenDrawer: () => void;
  theme: ThemeControl;
}) {
  const item = navItem(location.route);

  return (
    <header className="bg-background/80 sticky top-0 z-40 border-b backdrop-blur">
      <div className="mx-auto flex h-14 w-full max-w-[1400px] items-center gap-2 px-4 sm:gap-3 sm:px-6 lg:px-8">
        <Button
          variant="ghost"
          size="icon"
          className="lg:hidden"
          onClick={onOpenDrawer}
          aria-label="Open the menu"
        >
          <Menu />
        </Button>

        <div className="flex min-w-0 items-center gap-2">
          <item.icon className="text-muted-foreground size-4 shrink-0" />
          {/* On a phone the sidebar is a drawer and the page title is the only
              thing naming where you are — but it is the first thing to give up
              its room, because the controls beside it are what you came for. */}
          <span className="hidden truncate text-sm font-semibold sm:block">
            {item.label}
          </span>
        </div>

        <div className="ml-auto flex items-center gap-2">
          {SCOPED.includes(location.route) ? <ProjectPicker /> : null}
          {WINDOWED.includes(location.route) ? <WindowPicker /> : null}

          <Button
            variant="outline"
            size="sm"
            onClick={onOpenPalette}
            className="text-muted-foreground hidden gap-2 md:inline-flex"
          >
            <Search className="size-3.5" />
            <span>Search</span>
            <Kbd>⌘K</Kbd>
          </Button>

          <Tooltip label="Re-read everything on this page">
            <Button
              variant="ghost"
              size="icon"
              onClick={onRefresh}
              aria-label="Refresh"
            >
              <RefreshCw className={cn(refreshing && "animate-spin")} />
            </Button>
          </Tooltip>

          <Connection state={connection} />
          <ThemeMenu theme={theme} />
          <DaemonMenu status={status} />
        </div>
      </div>
    </header>
  );
}

function ProjectPicker() {
  const { project, setProject, projects } = useScope();

  if (projects.length === 0) return null;

  return (
    <Select
      value={project ?? "all"}
      onValueChange={(value) => setProject(value === "all" ? undefined : value)}
    >
      <SelectTrigger
        size="sm"
        className="w-28 sm:w-[9.5rem]"
        aria-label="Project"
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="all">All projects</SelectItem>
        {projects.map((candidate) => (
          <SelectItem key={candidate.id} value={candidate.id}>
            {candidate.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function WindowPicker() {
  const { days, setDays } = useScope();

  return (
    <div className="bg-muted hidden gap-0.5 rounded-lg p-0.5 sm:flex">
      {WINDOWS.map((option) => (
        <button
          key={option.days}
          onClick={() => setDays(option.days)}
          aria-pressed={days === option.days}
          className={cn(
            "rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
            days === option.days
              ? "bg-background text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function Connection({ state }: { state: ConnectionState }) {
  const label =
    state === "live"
      ? "Live — the daemon is streaming what it does"
      : state === "connecting"
        ? "Connecting to the daemon"
        : "Reconnecting — showing what was last read";

  return (
    <Tooltip label={label}>
      <Badge
        variant={state === "live" ? "success" : state === "connecting" ? "default" : "warning"}
        className="hidden sm:inline-flex"
      >
        <Dot
          tone={state === "live" ? "success" : state === "connecting" ? "muted" : "warning"}
          className={cn(state !== "live" && "animate-pulse")}
        />
        {state === "live" ? "Live" : state === "connecting" ? "Connecting" : "Offline"}
      </Badge>
    </Tooltip>
  );
}

const THEMES: { value: Theme; label: string; icon: React.ComponentType<{ className?: string }> }[] =
  [
    { value: "light", label: "Light", icon: Sun },
    { value: "dark", label: "Dark", icon: Moon },
    { value: "system", label: "System", icon: Laptop },
  ];

function ThemeMenu({ theme }: { theme: ThemeControl }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="Theme">
          {theme.resolved === "dark" ? <Moon /> : <Sun />}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Theme</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={theme.theme}
          onValueChange={(value) => theme.set(value as Theme)}
        >
          {THEMES.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              <option.icon />
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The daemon itself: what is answering, and how to stop it.
 *
 * Stopping is behind a confirmation because it is the one control here that
 * turns the dashboard off — the page it is on is served by the process it
 * stops. It is named for what it ends rather than for CtxC as a whole: MCP
 * servers an agent spawned keep running, and only `ctxc stop` reaches those.
 */
function DaemonMenu({ status }: { status: DaemonStatus | null }) {
  const [confirming, setConfirming] = React.useState(false);
  const [stopping, setStopping] = React.useState(false);

  const stop = async () => {
    setStopping(true);
    try {
      const result = await api.shutdown();
      toast.success("The daemon and dashboard are stopping.", {
        description:
          result.still_running > 0
            ? `${result.still_running} other CtxC process(es) still running. Run \`${result.stop_command}\` to end everything.`
            : "Start it again with `ctxc start --detach`.",
      });
      setConfirming(false);
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setStopping(false);
    }
  };

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Daemon">
            <Power />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-52">
          <DropdownMenuLabel>Daemon</DropdownMenuLabel>
          <div className="text-muted-foreground space-y-1 px-2 pb-2 text-xs">
            {status ? (
              <>
                <p className="tabular">pid {status.pid}</p>
                <p>
                  {status.watching} watched
                  {status.degraded > 0 ? `, ${status.degraded} polling` : ""}
                </p>
              </>
            ) : (
              <p>not answering</p>
            )}
          </div>
          <DropdownMenuSeparator />
          <DropdownMenuItem asChild>
            <Link to={href("/system")}>
              <Monitor /> System status
            </Link>
          </DropdownMenuItem>
          <DropdownMenuItem
            variant="destructive"
            onSelect={(event) => {
              event.preventDefault();
              setConfirming(true);
            }}
          >
            <CircleSlash /> Stop daemon &amp; dashboard
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Confirm
        open={confirming}
        onOpenChange={setConfirming}
        destructive
        pending={stopping}
        title="Stop the daemon and dashboard?"
        description={
          <span className="block space-y-2">
            <span className="block">
              This page is served by the daemon, so it will stop responding.
              Nothing is deleted, and <Code>ctxc start --detach</Code> brings it
              back.
            </span>
            <span className="block">
              It does not stop everything — an MCP server an agent spawned keeps
              running. Run <Code>ctxc stop</Code> in a terminal to end all of
              CtxC, and see System → Also running for what is left.
            </span>
          </span>
        }
        confirmLabel="Stop it"
        onConfirm={stop}
      />
    </>
  );
}
