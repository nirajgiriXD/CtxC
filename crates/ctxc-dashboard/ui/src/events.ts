/**
 * The live stream.
 *
 * The daemon publishes what it does over `WS /v1/events`, so the dashboard
 * updates as things happen rather than polling and always being a second
 * behind. The socket is the notification, not the source of truth: an event
 * says "something changed", and the affected view re-reads it from the API.
 * That way a dropped frame costs a delay, never a wrong number.
 */

import { useEffect, useRef, useState } from "react";
import type { MetricEvent, WatchReport } from "./api";
import { token } from "./api";

export type StreamEvent =
  | { type: "operation"; at: number; event: MetricEvent }
  | { type: "project"; at: number; id: string; name: string; status: string }
  | { type: "watching"; at: number; projects: WatchReport[] }
  | { type: "notice"; at: number; level: string; message: string }
  | { type: "lagged"; at: number; missed: number };

export type ConnectionState = "connecting" | "live" | "offline";

/** How long to wait before reconnecting, and the ceiling it backs off to. */
const FIRST_RETRY_MS = 500;
const MAX_RETRY_MS = 10_000;

interface Stream {
  state: ConnectionState;
  /** Increments on every event, so views can re-read when it changes. */
  revision: number;
  /** The most recent events, newest first. */
  recent: MetricEvent[];
}

/**
 * Subscribe to the daemon for as long as the page is open.
 *
 * Reconnects with backoff, because the daemon being restarted underneath an
 * open dashboard is ordinary rather than exceptional.
 */
export function useEventStream(keep = 50): Stream {
  const [state, setState] = useState<ConnectionState>("connecting");
  const [revision, setRevision] = useState(0);
  const [recent, setRecent] = useState<MetricEvent[]>([]);

  // Held in a ref so the cleanup can close whatever is currently open, without
  // the effect depending on it and reconnecting on every render.
  const socket = useRef<WebSocket | null>(null);
  const retry = useRef(FIRST_RETRY_MS);
  const timer = useRef<number | null>(null);
  const closed = useRef(false);

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
      };

      ws.onmessage = (message) => {
        let event: StreamEvent;
        try {
          event = JSON.parse(message.data as string) as StreamEvent;
        } catch {
          return;
        }

        if (event.type === "operation") {
          setRecent((current) => [event.event, ...current].slice(0, keep));
        }
        // Lagging means the dashboard missed events, so every view should
        // re-read rather than trust what it has.
        setRevision((current) => current + 1);
      };

      ws.onclose = () => {
        socket.current = null;
        if (closed.current) return;

        setState("offline");
        timer.current = window.setTimeout(connect, retry.current);
        retry.current = Math.min(retry.current * 2, MAX_RETRY_MS);
      };

      // `onclose` always follows `onerror`, so reconnection is handled there
      // and this only stops the default console noise from being the only
      // sign that something went wrong.
      ws.onerror = () => ws.close();
    };

    connect();

    return () => {
      closed.current = true;
      if (timer.current !== null) window.clearTimeout(timer.current);
      socket.current?.close();
    };
  }, [keep]);

  return { state, revision, recent };
}
