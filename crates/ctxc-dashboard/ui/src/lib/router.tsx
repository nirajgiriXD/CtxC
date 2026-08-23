/**
 * Routing, in the hundred lines it actually takes.
 *
 * The dashboard has eight screens and no data loading tied to navigation, so a
 * router library would be more configuration than code. What is needed is a
 * current location, a way to change it, and links that behave like links —
 * middle click, ctrl-click and the back button all working without special
 * cases.
 *
 * Locations live in the fragment (`#/projects/abc`). The daemon serves the
 * dashboard from its own root and falls back to the entry point for unknown
 * paths, so history routing would work too; the fragment is chosen because it
 * survives being opened from a file, a proxy, or any prefix the dashboard might
 * be mounted under later.
 */

import * as React from "react";

/** The routes the dashboard has. Anything else is not a page. */
export const ROUTES = [
  "/",
  "/projects",
  "/activity",
  "/performance",
  "/context",
  "/commands",
  "/settings",
  "/system",
] as const;

export type Route = (typeof ROUTES)[number];

/** A parsed location: which page, and what it was pointed at. */
export interface Location {
  /** The page, always one of [`ROUTES`]. */
  route: Route;
  /** The rest of the path, e.g. the project id in `/projects/abc`. */
  detail?: string;
  /** The full path, for comparing and for `key`s. */
  path: string;
}

/** Read the current fragment, falling back to the overview. */
export function current(): Location {
  const raw = window.location.hash.replace(/^#/, "") || "/";
  return parse(raw);
}

export function parse(raw: string): Location {
  const path = raw.startsWith("/") ? raw : `/${raw}`;
  const [, first = "", ...rest] = path.split("/");
  const route = (`/${first}` as Route);

  if (!ROUTES.includes(route)) {
    // An unknown route is a typo or a stale bookmark, not a page. The overview
    // is the honest answer: it is what the dashboard opens on.
    return { route: "/", path: "/" };
  }

  const detail = rest.filter(Boolean).map(decodeURIComponent).join("/");
  return { route, detail: detail || undefined, path };
}

/** Build a path for a route and an optional detail segment. */
export function href(route: Route, detail?: string): string {
  const base = route === "/" ? "/" : route;
  return detail ? `${base}/${encodeURIComponent(detail)}` : base;
}

/** Go somewhere. Pushes history, so the back button works. */
export function navigate(path: string) {
  const target = `#${path.startsWith("/") ? path : `/${path}`}`;
  if (window.location.hash === target) return;
  window.location.hash = target;
}

/** Replace the current entry instead of adding one, for redirects. */
export function replace(path: string) {
  const url = new URL(window.location.href);
  url.hash = path.startsWith("/") ? path : `/${path}`;
  window.history.replaceState({}, "", url.toString());
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}

/** Subscribe to the current location. */
export function useLocation(): Location {
  const [location, setLocation] = React.useState(current);

  React.useEffect(() => {
    const update = () => setLocation(current());
    window.addEventListener("hashchange", update);
    return () => window.removeEventListener("hashchange", update);
  }, []);

  // Every page starts at the top: arriving on a long settings screen already
  // scrolled halfway down is disorienting.
  React.useEffect(() => {
    window.scrollTo({ top: 0 });
  }, [location.path]);

  return location;
}

/**
 * A link.
 *
 * A real `<a href="#/...">`, so the browser handles opening in a new tab, the
 * status bar shows where it goes, and screen readers announce it as a link.
 */
export function Link({
  to,
  children,
  ...props
}: { to: string } & Omit<React.ComponentProps<"a">, "href">) {
  return (
    <a href={`#${to}`} {...props}>
      {children}
    </a>
  );
}
