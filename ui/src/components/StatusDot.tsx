// New for agent-router: what a session is doing, as a dot and as the header's pill. Custom because
// Carbon's IconIndicator and ShapeIndicator are preview exports in Carbon's own status colours,
// with no hollow ring, halo or pulse (docs/web-console-carbon.md, "Why Carbon, and what it does
// not cover"). Drawn from the live view's `.dot` and `#stat` (assets/live.html).
import styles from './StatusDot.module.scss';

/// The router's tones (SE-C2). The session model decides a row's tone; this only draws it, the
/// same way in a list row, a project heading, a tab and the header (SE-C5).
const TONES = ['busy', 'attention', 'warn', 'error', 'idle', 'closed'] as const;
export type Tone = (typeof TONES)[number];

/// The phones' words for a tone, said when the caller has none of its own (SE-C20, SE-C63).
const WORDS: Readonly<Record<Tone, string>> = {
  busy: 'working',
  attention: 'waiting for you',
  warn: 'stalled',
  error: 'failed',
  idle: 'idle',
  closed: 'ended',
};

const isTone = (tone: unknown): tone is Tone => typeof tone === 'string' && (TONES as readonly string[]).includes(tone);

/// What a dot says. The caller's words when it has any, else its tone's, and for a tone nobody
/// knows "status unknown": never empty, and never "undefined" (SE-C21).
function wordsFor(tone: unknown, words: string | undefined): string {
  const said = words?.trim();
  if (said) return said;
  return isTone(tone) ? WORDS[tone] : 'status unknown';
}

/// One session state as a dot.
///
/// The words are its accessible name, so a dot standing alone in a heading or a tab is never
/// colour only (SE-C20); inside a button they become part of the button's name. A dot drawn beside
/// its own words is `decorative`, hidden from a screen reader, which would otherwise hear them
/// twice.
export function StatusDot({
  tone,
  label,
  live = false,
  decorative = false,
  className,
}: {
  /// The router's tone. Anything else, a newer router's word included, is the plain neutral dot.
  tone: string | null | undefined;
  /// What the session is doing, in words. Its tone's word when absent.
  label?: string;
  /// Busy or waiting on you: the halo, for a list row and a project heading only (SE-C9).
  live?: boolean;
  decorative?: boolean;
  className?: string;
}) {
  return (
    <span
      className={[styles.dot, className].filter(Boolean).join(' ')}
      data-tone={isTone(tone) ? tone : undefined}
      data-live={live ? '' : undefined}
      role={decorative ? undefined : 'img'}
      aria-label={decorative ? undefined : wordsFor(tone, label)}
      aria-hidden={decorative ? true : undefined}
    />
  );
}

/// The header's pill: the pip and what the session is doing right now, as the stream says it.
///
/// A `status` the reader can find, but never a live region: the words carry a duration that
/// changes every second, and a screen reader would be read each second of it. A tab's dot and
/// the header's pip never wear the halo (SE-C9), so the pill takes no `live`.
export function StatusPill({
  tone,
  text,
  stale = false,
  className,
}: {
  tone: string | null | undefined;
  /// The status line (SE-C62). Empty, it is the tone's word (SE-C63).
  text?: string;
  /// The stream no longer vouches for its last word — connecting, reconnecting, silent — so the
  /// pip has no tone and the words are neutral italics (SE-C61, SE-C64).
  stale?: boolean;
  className?: string;
}) {
  const words = wordsFor(stale ? undefined : tone, text);
  return (
    <span
      className={[styles.pill, className].filter(Boolean).join(' ')}
      role="status"
      aria-live="off"
      aria-label={words}
      data-stale={stale ? '' : undefined}
    >
      <StatusDot tone={stale ? undefined : tone} decorative />
      <span className={styles.words}>{words}</span>
    </span>
  );
}
