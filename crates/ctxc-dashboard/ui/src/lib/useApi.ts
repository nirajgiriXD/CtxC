/**
 * Reading from and writing to the daemon.
 *
 * Loading, failed and loaded are kept apart rather than collapsed into "data or
 * nothing": an empty dashboard and a broken one must never look the same, and a
 * stale number must never be shown as if it were current.
 *
 * A refresh keeps the previous data on screen and raises `refreshing` instead of
 * `loading`, so the live stream can re-read a panel every few seconds without
 * the page dissolving into skeletons each time.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { RequestFailed } from "./api";

export interface Query<T> {
  data: T | null;
  error: RequestFailed | Error | null;
  /** True only until the first answer arrives. */
  loading: boolean;
  /** True while a later read is in flight, with data still on screen. */
  refreshing: boolean;
  /** When the data on screen was fetched. */
  fetchedAt: number | null;
  reload: () => void;
}

/**
 * Run `fetcher` now, and again whenever `deps` change.
 *
 * `revision` from the event stream belongs in `deps`: that is how a change the
 * daemon announced becomes a re-read rather than a guess at what changed.
 */
export function useApi<T>(
  fetcher: () => Promise<T>,
  deps: React.DependencyList,
): Query<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<RequestFailed | Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [fetchedAt, setFetchedAt] = useState<number | null>(null);
  const [attempt, setAttempt] = useState(0);
  const loaded = useRef(false);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  const run = useCallback(fetcher, deps);

  useEffect(() => {
    // A response that arrives after the component moved on must not be written
    // into state it no longer owns.
    let current = true;
    if (loaded.current) setRefreshing(true);

    run()
      .then((value) => {
        if (!current) return;
        setData(value);
        setError(null);
        setFetchedAt(Date.now());
      })
      .catch((cause: unknown) => {
        if (!current) return;
        setError(cause instanceof Error ? cause : new Error(String(cause)));
      })
      .finally(() => {
        if (!current) return;
        loaded.current = true;
        setLoading(false);
        setRefreshing(false);
      });

    return () => {
      current = false;
    };
  }, [run, attempt]);

  const reload = useCallback(() => setAttempt((n) => n + 1), []);

  return { data, error, loading, refreshing, fetchedAt, reload };
}

export interface Mutation<Args extends unknown[], Result> {
  run: (...args: Args) => Promise<Result | undefined>;
  /** The last failure, kept so a form can show it next to the control. */
  error: RequestFailed | Error | null;
  clearError: () => void;
  pending: boolean;
}

/**
 * Perform one action against the daemon.
 *
 * Nothing is applied optimistically. The daemon announces what changed and the
 * re-read that follows shows what actually happened, which matters when a
 * directory was deleted underneath a project or an edit was refused.
 */
export function useMutation<Args extends unknown[], Result>(
  action: (...args: Args) => Promise<Result>,
  options: { onDone?: (result: Result) => void } = {},
): Mutation<Args, Result> {
  const [error, setError] = useState<RequestFailed | Error | null>(null);
  const [pending, setPending] = useState(false);
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const { onDone } = options;
  const done = useRef(onDone);
  done.current = onDone;

  const run = useCallback(
    async (...args: Args) => {
      setPending(true);
      setError(null);
      try {
        const result = await action(...args);
        done.current?.(result);
        return result;
      } catch (cause) {
        if (alive.current) {
          setError(cause instanceof Error ? cause : new Error(String(cause)));
        }
        return undefined;
      } finally {
        if (alive.current) setPending(false);
      }
    },
    [action],
  );

  const clearError = useCallback(() => setError(null), []);

  return { run, error, clearError, pending };
}

/** The hint on a failure, when the daemon sent one. */
export function hintOf(error: Error | null): string | undefined {
  return error && "hint" in error
    ? (error as RequestFailed).hint
    : undefined;
}
