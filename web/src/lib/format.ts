/**
 * Timestamps for the authoring app.
 *
 * Local time here, unlike the public site's UTC. The public site renders in UTC
 * so the edge cache never has to vary by reader; this one has exactly one
 * reader, and "3 hours ago" in their own zone is what they actually want.
 */

const ABSOLUTE = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "short",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "just now", "12m", "3h", "5d", then an absolute date. */
export function relative(millis: number, now = Date.now()): string {
  const elapsed = now - millis;

  if (elapsed < 0) return "scheduled";
  if (elapsed < MINUTE) return "just now";
  if (elapsed < HOUR) return `${Math.floor(elapsed / MINUTE)}m`;
  if (elapsed < DAY) return `${Math.floor(elapsed / HOUR)}h`;
  if (elapsed < 7 * DAY) return `${Math.floor(elapsed / DAY)}d`;

  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    ...(new Date(millis).getFullYear() === new Date(now).getFullYear()
      ? {}
      : { year: "numeric" }),
  }).format(millis);
}

/** The full timestamp, for a title attribute. */
export function absolute(millis: number): string {
  return ABSOLUTE.format(millis);
}
