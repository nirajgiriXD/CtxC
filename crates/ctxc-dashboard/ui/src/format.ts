/**
 * Turning numbers into something a person can read at a glance.
 *
 * Every token count here is an estimate, and every cost derived from one
 * inherits that. The formatters keep the numbers honest — they never round a
 * negative saving away, and they never turn "not measured" into a zero.
 */

/** 84,203,118 -> "84.2M". Compact, because these numbers get large. */
export function compact(value: number): string {
  const sign = value < 0 ? "-" : "";
  const size = Math.abs(value);

  if (size < 1_000) return `${sign}${size}`;
  if (size < 1_000_000) return `${sign}${(size / 1_000).toFixed(1)}K`;
  if (size < 1_000_000_000) return `${sign}${(size / 1_000_000).toFixed(1)}M`;
  return `${sign}${(size / 1_000_000_000).toFixed(1)}B`;
}

/** 1234567 -> "1,234,567". For places where the exact figure matters. */
export function exact(value: number): string {
  return value.toLocaleString("en-US");
}

/** 0.6234 -> "62.3%". */
export function percent(ratio: number): string {
  return `${(ratio * 100).toFixed(1)}%`;
}

/** Bytes, for the index and the database. */
export function bytes(value: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit + 1 < units.length) {
    size /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${size.toFixed(1)} ${units[unit]}`;
}

/** A duration in milliseconds, at whatever scale reads best. */
export function duration(ms: number): string {
  if (ms < 1_000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)} s`;
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)} min`;
  return `${(ms / 3_600_000).toFixed(1)} h`;
}

/** How long ago something happened: "2 minutes ago". */
export function ago(epochMillis: number): string {
  const seconds = Math.max(0, (Date.now() - epochMillis) / 1000);

  if (seconds < 10) return "just now";
  if (seconds < 60) return `${Math.round(seconds)} seconds ago`;

  const minutes = seconds / 60;
  if (minutes < 60) return plural(Math.round(minutes), "minute");

  const hours = minutes / 60;
  if (hours < 24) return plural(Math.round(hours), "hour");

  return plural(Math.round(hours / 24), "day");
}

function plural(count: number, unit: string): string {
  return `${count} ${unit}${count === 1 ? "" : "s"} ago`;
}

/** The wall-clock time of an event, for the activity feed. */
export function clock(epochMillis: number): string {
  return new Date(epochMillis).toLocaleTimeString("en-GB", { hour12: false });
}

/** A bucket label on a chart axis. */
export function bucketLabel(
  epochMillis: number,
  granularity: "hour" | "day",
): string {
  const at = new Date(epochMillis);
  return granularity === "hour"
    ? at.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })
    : at.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/**
 * A cost figure, or an honest absence.
 *
 * The daemon omits this entirely when no rate is configured, and inventing a
 * "$0.00" here would read as "this saved nothing" rather than "nobody said what
 * a token costs".
 */
export function cost(
  estimate: { amount: number; currency: string } | undefined,
): string | null {
  if (!estimate) return null;
  return `${estimate.currency} ${estimate.amount.toFixed(2)}`;
}
