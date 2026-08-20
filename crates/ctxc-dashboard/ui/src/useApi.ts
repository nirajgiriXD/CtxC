/**
 * Reading from the daemon, with the three states a read actually has.
 *
 * Loading, failed, and loaded are kept apart rather than collapsed into
 * "data or nothing": an empty dashboard and a broken one must never look the
 * same, and a stale number must never be shown as if it were current.
 */

import { useCallback, useEffect, useState } from "react";
import { RequestFailed } from "./api";

export interface Query<T> {
  data: T | null;
  error: RequestFailed | Error | null;
  /** True only on the first load; a refresh keeps the old data on screen. */
  loading: boolean;
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
  const [attempt, setAttempt] = useState(0);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  const run = useCallback(fetcher, deps);

  useEffect(() => {
    // A response that arrives after the component moved on must not be
    // written into state it no longer owns.
    let current = true;

    run()
      .then((value) => {
        if (!current) return;
        setData(value);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (!current) return;
        setError(cause instanceof Error ? cause : new Error(String(cause)));
      })
      .finally(() => {
        if (current) setLoading(false);
      });

    return () => {
      current = false;
    };
  }, [run, attempt]);

  const reload = useCallback(() => setAttempt((n) => n + 1), []);

  return { data, error, loading, reload };
}
