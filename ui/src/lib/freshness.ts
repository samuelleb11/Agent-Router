// Copied from Northstar ui/src/lib/freshness.ts at e9c1589.
// Changed: the poll is the router's 5 s rather than Northstar's 3 s, and the stream sweep is gone.

/// How often a view re-reads. The router's dashboard has polled every 5 s since it was
/// written, and its phones on the same beat; the console keeps it, so a figure here and the same
/// figure on a phone are never a beat apart by design.
export const POLL_MS = 5000;

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/// Render a timestamp as "just now" / "12s ago" / "3h ago" / "2d ago", so a view can say
/// how current it is. Rolls up at each unit rather than stopping at minutes: a session last
/// seen yesterday reads "1d ago", not "1440m ago".
///
/// A display that cannot say when it was measured has the same defect as a count that
/// cannot say what it counted.
///
/// **`never` is only right when the caller knows nothing happened.** This answers it for any
/// missing value, which is a fact about the CALLER's data and not one this function can
/// establish — an optional field the server never sent is absent for a reason the wire does
/// not carry. A caller holding an optional wire time decides between "never" and "time
/// unknown" itself, from something that states it, before asking for a relative time.
export function since(at: number | null | undefined): string {
  if (at === null || at === undefined) return 'never';
  const s = Math.round((Date.now() - at) / 1000);
  if (s < 2) return 'just now';
  if (s < MINUTE) return `${s}s ago`;
  const m = Math.round(s / MINUTE);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(s / HOUR);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(s / DAY)}d ago`;
}

/// The same, for an ISO timestamp off the wire.
export function sinceIso(iso: string | null | undefined): string {
  if (!iso) return 'never';
  const t = Date.parse(iso);
  return Number.isNaN(t) ? 'never' : since(t);
}

/// Hold what was last read, and label it — never blank it.
///
/// Losing what you were reading to a transient blip is worse than seeing it with its age on
/// it. In React this is what TanStack Query already does — a failed refetch leaves the last
/// successful `data` in place and raises `isError` beside it — so a view reads BOTH and says
/// "showing what was read 12s ago" rather than replacing the screen with an error. This helper
/// is the shape of that decision for a view to render, so the rule lives in one place rather
/// than in every screen.
export interface Held<T> {
  /// What to render. Null only when nothing has ever been read.
  readonly value: T | null;
  /// When `value` was read, for `since`.
  readonly at: number | null;
  /// True when the most recent read failed and `value` is therefore stale.
  readonly stale: boolean;
}

export function held<T>(data: T | undefined, isError: boolean, dataUpdatedAt: number): Held<T> {
  return {
    value: data ?? null,
    at: data === undefined || dataUpdatedAt === 0 ? null : dataUpdatedAt,
    stale: isError && data !== undefined,
  };
}
