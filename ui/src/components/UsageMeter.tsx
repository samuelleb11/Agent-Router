// New for agent-router: one quota window as a meter. Custom because Carbon's ProgressBar is
// `role=progressbar` with an `aria-live` helper that would re-announce on every five-second poll,
// where a quota needs `role=meter` and the 70 % and 90 % thresholds (docs/web-console-carbon.md,
// "Why Carbon, and what it does not cover"). Drawn from the dashboard's `renderQuota` meters
// (assets/index.html).
import { useId, type ReactNode } from 'react';
import styles from './UsageMeter.module.scss';

/// Amber from here (OV-C57).
const WARN_AT = 70;
/// Red from here (OV-C57).
const BAD_AT = 90;

type Level = 'ok' | 'warn' | 'bad' | 'none' | 'expired';

/// One quota window: "N% used · resets in …" over a bar capped at 100 %.
///
/// A meter to assistive technology, named by its window and valued by the same words the eye
/// reads, and never a live region: the reading changes on every poll, and a screen reader would
/// be read each one. With no reading there is nothing to measure, so there is no meter either —
/// a meter at zero would say "0 %" to a screen reader where the eye reads "—" (OV-C58).
export function UsageMeter({
  label,
  used,
  resets,
  resetsAt,
  expired,
  tone = 'accent',
  children,
}: {
  /// The window: "5-hour window", "7-day window".
  label: string;
  /// Percent of the window used, 0–100. Null or absent when there is no reading.
  used: number | null | undefined;
  /// When it resets, in the formatters' words: "resets in 2h 10m".
  resets?: string;
  /// The exact reset time, shown as text under the bar (OV-C63).
  resetsAt?: string;
  /// The window has rolled over: the sentence that says so, "window rolled over 3 h ago · nothing
  /// seen since". Its old utilisation is then never shown, whatever `used` holds (OV-C72).
  expired?: string;
  /// The window's own colour below 70 %: the 5-hour window Carbon's blue, the 7-day slate (OV-C62).
  tone?: 'accent' | 'slate';
  /// What goes under the meter: the window's forecast line.
  children?: ReactNode;
}) {
  const labelId = useId();
  const reading = expired === undefined && typeof used === 'number' && Number.isFinite(used);
  const capped = reading ? Math.min(100, Math.max(0, used)) : 0;
  const shown = Math.round(capped);
  const level: Level = expired !== undefined ? 'expired' : !reading ? 'none' : shown >= BAD_AT ? 'bad' : shown >= WARN_AT ? 'warn' : 'ok';
  const words = expired ?? (reading ? [`${shown}% used`, resets].filter(Boolean).join(' · ') : '—');

  return (
    <div className={styles.meter} data-level={level} data-tone={tone}>
      <div className={styles.head}>
        <span id={labelId} className={styles.label}>
          {label}
        </span>
        <span className={styles.value}>{words}</span>
      </div>
      {reading ? (
        <div
          className={styles.track}
          role="meter"
          aria-labelledby={labelId}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={shown}
          aria-valuetext={words}
        >
          <span className={styles.fill} style={{ inlineSize: `${capped}%` }} />
        </div>
      ) : (
        <div className={styles.track} aria-hidden="true" />
      )}
      {resetsAt && level !== 'expired' && <span className={styles.at}>resets at {resetsAt}</span>}
      {children}
    </div>
  );
}
