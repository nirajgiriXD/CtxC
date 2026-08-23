/**
 * The live stream.
 *
 * The daemon publishes what it does over `WS /v1/events`, so the dashboard
 * updates as things happen rather than polling and always being a second
 * behind. The socket is the notification, not the source of truth: an event
 * says "something changed", and the affected view re-reads it from the API.
 * That way a dropped frame costs a delay, never a wrong number.
 *
 * Revisions are counted per subject rather than globally. A page showing
 * settings has no reason to re-read because a file was indexed, and a table of
 * projects should not flicker every time an optimization is recorded.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { MetricEvent, WatchReport } from "./api";
import { token } from "./api";

export type StreamEvent =
  | { type: "operation"; at: number; event: MetricEvent }
  | { type: "project"; at: number; id: string; name: string; status: string }
  | { type: "watching"; at: number; projects: WatchReport[] }
  | { type: "config"; at: number; changed: string[] }
  | { type: "notice"; at: number; level: string; message: string }
  | { type: "lagged"; at: number; missed: number };

export type ConnectionState = "connecting" | "live" | "offline";

/** How long to wait before reconnecting, and the ceiling it backs off to. */
const FIRST_RETRY_MS = 500;
const MAX_RETRY_MS = 10_000;

/** What each counter tracks, so a view can depend on only what concerns it. */
export interface Revisions {
  /** Operations: indexing, optimizing, searching. */
  operations: number;
  /** Projects added, paused, resumed or removed. */
  projects: number;
  /** What the supervisor is watching. */
  watching: number;
  /** The configuration file. */
  config: number;
  /** Any event at all, for panels that summarise everything. */
  any: number;
}

const NOTHING_YET: Revisions = {
  operations: 0,
  projects: 0,
  watching: 0,
  config: 0,
  any: 0,
};

export interface Stream {
  state: ConnectionState;
  revisions: Revisions;
  /** The most recent operations, newest first. */
  recent: MetricEvent[];
  /** Set when the daemon told us we missed events. */
  missed: number;
  /** Ask every panel to re-read, for the manual refresh control. */
  refresh: () => void;
}

/**
 * Subscribe to the daemon for as long as the page is open.
 *
 * Reconnects with backoff, because the daemon being restarted underneath an
 * open dashboard is ordinary rather than exceptional.
 */
export function useEventStream(keep = 60): Stream {
  const [state, setState] = useState<ConnectionState>("connecting");
  const [revisions, setRevisions] = useState<Revisions>(NOTHING_YET);
  const [recent, setRecent] = useState<MetricEvent[]>([]);
  const [missed, setMissed] = useState(0);

  // Held in refs so the cleanup can close whatever is currently open, without
  // the effect depending on it and reconnecting on every render.
  const socket = useRef<WebSocket | null>(null);
  const retry = useRef(FIRST_RETRY_MS);
  const timer = useRef<number | null>(null);
  const closed = useRef(false);

  const bump = useCallback((...subjects: (keyof Revisions)[]) => {
    setRevisions((current) => {
      const next = { ...current, any: current.any + 1 };
      for (const subject of subjects) next[subject] = current[subject] + 1;
      return next;
    });
  }, []);

  const refresh = useCallback(
    () => bump("operations", "projects", "watching", "config"),
    [bump],
  );

  useEffect(() => {
    closed.current = false;

    const connect = () => {
      if (closed.current) return;

      const scheme = window.location.protocol === "https:" ? "wss" : "ws";
      const url = `${scheme}://${window.location.host}/v1/events?token=${encodeURIComponent(token)}`;
      const ws = new WebSocket(url);
      socket.current = ws;

      ws.onopen = () => {
        retry.current = FIRST_RETRY_MS;
        setState("live");
        // A reconnected socket has no idea what happened while it was gone, so
        // everything on screen is suspect until it has been read again.
        bump("operations", "projects", "watching", "config");
      };

      ws.onmessage = (message) => {
        let event: StreamEvent;
        try {
          event = JSON.parse(message.data as string) as StreamEvent;
        } catch {
          return;
        }

        switch (event.type) {
          case "operation":
            setRecent((current) => [event.event, ...current].slice(0, keep));
            bump("operations");
            break;
          case "project":
            bump("projects", "operations");
            break;
          case "watching":
            bump("watching");
            break;
          case "config":
            bump("config");
            break;
          case "lagged":
            // Lagging means the dashboard missed events, so every view should
            // re-read rather than trust what it has.
            setMissed((total) => total + event.missed);
            bump("operations", "projects", "watching", "config");
            break;
          default:
            bump();
        }
      };

      ws.onclose = () => {
        socket.current = null;
        if (closed.current) return;

        setState("offline");
        timer.current = window.setTimeout(connect, retry.current);
        retry.current = Math.min(retry.current * 2, MAX_RETRY_MS);
      };

      // `onclose` always follows `onerror`, so reconnection is handled there
      // and this only stops the default console noise from being the only sign
      // that something went wrong.
      ws.onerror = () => ws.close();
    };

    connect();

    return () => {
      closed.current = true;
      if (timer.current !== null) window.clearTimeout(timer.current);
      socket.current?.close();
    };
  }, [keep, bump]);

  return { state, revisions, recent, missed, refresh };
}
