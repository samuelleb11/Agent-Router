// New for agent-router: one quota window's burn-down. Custom because the dashed projection ray, the
// single crossing marker and the quiet-to-busy cone carry the chart's meaning, and Carbon Charts
// draws none of them (docs/web-console-carbon.md, "Why Carbon, and what it does not cover"). Drawn
// from the dashboard's `runwaySvg` and `renderRunway` (assets/index.html).
import { DataTip, type Tip } from './DataTip';
import styles from './BurnDown.module.scss';

/// The forecast's three-state answer (OV-C68).
export type Verdict = 'crosses' | 'tight' | 'resets-first';

/// Where one week's pace lands: on the 100 % line at `crossing`, or at `end` when the window
/// resets first.
export interface Landing {
  /// When it reaches 100 %, epoch milliseconds; null when it does not.
  readonly crossing: number | null;
  /// Where it ends at the reset, 0–1, when it does not cross.
  readonly end: number | null;
}

/// One window of the forecast, in milliseconds and fractions of the limit.
export interface Burn extends Landing {
  /// When the window opened.
  readonly start: number | null;
  /// When it resets.
  readonly reset: number | null;
  /// Used now, 0–1.
  readonly used: number;
  /// What was seen so far: time and fraction used.
  readonly series?: readonly { readonly t: number; readonly u: number }[];
  /// Null from a router that predates the band, which is read by the old boolean: a crossing
  /// means crosses, none means resets first (OV-C71).
  readonly verdict: Verdict | null;
  /// The quiet and the busy week, present only when the duty band has width (OV-C83).
  readonly band?: { readonly quiet: Landing; readonly busy: Landing } | null;
  /// The window has rolled over: nothing current to draw (OV-C72).
  readonly expired?: boolean;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const pct = (u: number) => `${Math.round(clamp(u, 0, 1) * 100)}%`;
const time = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });

/// One window drawn across its own life: what was seen left of now, and a dashed projection right
/// of it, to its crossing on the 100 % line when there is one, otherwise to the reset at the level
/// it ends (OV-C80, OV-C81).
///
/// A window that cannot be drawn is not: expired, absent, with no start, or a reset not after its
/// start draws nothing rather than dividing by zero, and every time outside the window is held to
/// its edges (OV-C84). Its times and figures are text under the chart, and its accessible name
/// says them too, so none waits for a hover (OV-C63).
export function BurnDown({
  label,
  burn,
  now = Date.now(),
  tone = 'accent',
  formatTime = time,
  className,
}: {
  /// The window: "7-day window".
  label: string;
  burn: Burn | null | undefined;
  /// Now, epoch milliseconds.
  now?: number;
  /// The window's own colour (OV-C62): the 5-hour window Carbon's blue, the 7-day slate.
  tone?: 'accent' | 'slate';
  formatTime?: (ms: number) => string;
  className?: string;
}) {
  if (!burn || burn.expired || burn.start === null || burn.reset === null || !Number.isFinite(burn.start) || !Number.isFinite(burn.reset)) return null;
  const { start, reset } = burn;
  if (!(reset > start)) return null;

  const x = (t: number) => 100 * clamp((t - start) / (reset - start), 0, 1);
  const y = (u: number) => 100 - 100 * clamp(u, 0, 1);
  const at = clamp(now, start, reset);
  const verdict: Verdict = burn.verdict ?? (burn.crossing !== null ? 'crosses' : 'resets-first');

  /// Where a pace lands, as a point: the crossing on the 100 % line, else the reset at its end.
  const land = (l: Landing): [number, number] => (l.crossing !== null ? [x(l.crossing), y(1)] : [x(reset), y(l.end ?? burn.used)]);
  const said = (l: Landing) => (l.crossing !== null ? `reaches 100% ${formatTime(l.crossing)}` : `ends ≈${pct(l.end ?? burn.used)} at the reset`);

  const seen = [...(burn.series ?? []).map((p): [number, number] => [x(p.t), y(p.u)]), [x(at), y(burn.used)] as [number, number]];
  const seenPath = seen.map(([px, py], i) => `${i ? 'L' : 'M'}${px.toFixed(2)},${py.toFixed(2)}`).join('');
  const from = `${x(at).toFixed(2)},${y(burn.used).toFixed(2)}`;
  const target = land(burn);
  const cone = burn.band
    ? (() => {
        const busy = land(burn.band.busy);
        const quiet = land(burn.band.quiet);
        return `M${from}L${busy[0].toFixed(2)},${busy[1].toFixed(2)}L${x(reset).toFixed(2)},${busy[1].toFixed(2)}L${x(reset).toFixed(2)},${quiet[1].toFixed(2)}L${quiet[0].toFixed(2)},${quiet[1].toFixed(2)}Z`;
      })()
    : null;

  const opened = formatTime(start);
  const resets = `resets ${formatTime(reset)}`;
  const usedNow = `${pct(burn.used)} used now`;
  const ray = said(burn);
  const weeks = burn.band ? `a quiet week ${said(burn.band.quiet)}, a busy one ${said(burn.band.busy)}` : null;
  const named = `${label} · burn-down: opened ${opened}, ${resets}; ${usedNow}, ${ray}${weeks ? `; ${weeks}` : ''}`;

  const tip = (): Tip => ({
    title: `${label} — burn-down`,
    rows: [
      { value: pct(burn.used), label: 'used now', colour: tone === 'slate' ? '--ar-tok' : '--cds-interactive', mark: 'line' },
      { value: ray, label: 'projection', colour: verdict === 'crosses' ? '--ar-bad-fill' : verdict === 'tight' ? '--ar-warn-fill' : '--cds-icon-secondary', mark: 'line' },
      weeks ? { value: weeks, label: '' } : null,
      { value: opened, label: 'window opened' },
      { value: formatTime(reset), label: 'window resets' },
    ],
  });

  return (
    <figure className={[styles.burn, className].filter(Boolean).join(' ')}>
      <figcaption className={styles.caption}>{label} · burn-down</figcaption>
      <DataTip label={named} tip={tip}>
        <div className={styles.chart}>
          <div className={styles.axis} aria-hidden="true">
            <span>100%</span>
            <span>0</span>
          </div>
          <div className={styles.plot}>
            <svg className={styles.svg} viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true" focusable="false">
              <line className={styles.rule} x1={0} x2={100} y1={0} y2={0} />
              <line className={styles.rule} x1={0} x2={100} y1={100} y2={100} />
              {cone && <path className={styles.cone} data-verdict={verdict} d={cone} />}
              <line className={styles.now} x1={x(at)} x2={x(at)} y1={0} y2={100} />
              <path className={styles.seen} data-tone={tone} d={seenPath} />
              <path className={styles.ray} data-verdict={verdict} d={`M${from}L${target[0].toFixed(2)},${target[1].toFixed(2)}`} />
            </svg>
            {burn.crossing !== null && (
              <span className={styles.marker} data-verdict={verdict} style={{ insetInlineStart: `${target[0]}%`, insetBlockStart: `${target[1]}%` }} />
            )}
          </div>
        </div>
      </DataTip>
      <div className={styles.ends}>
        <span>{opened}</span>
        <span>{resets}</span>
      </div>
      <p className={styles.facts}>
        <span>{`${usedNow} · ${ray}`}</span>
        {weeks && <span>{weeks}</span>}
      </p>
    </figure>
  );
}
