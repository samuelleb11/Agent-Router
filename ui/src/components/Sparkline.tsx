// New for agent-router: a sparkline strip. Custom because it is 68 px tall and marks only its last
// point, where a Carbon LineChart needs 100–120 px and cannot (docs/web-console-carbon.md, "Why
// Carbon, and what it does not cover"). Drawn from the dashboard's `sparkSvg` and its quota history
// (assets/index.html), whose figures followed the pointer only.
import { useRef, useState } from 'react';
import type { ArToken } from '~/lib/concepts';
import { DataTip, type Tip } from './DataTip';
import styles from './Sparkline.module.scss';

/// The strip's height in CSS pixels (Sparkline.module.scss): a bucket above zero is at least one of
/// them tall (OV-C194).
const STRIP = 68;
const PIXEL = 100 / STRIP;

export interface Series {
  readonly key: string;
  /// What it counts: "prompt tokens", "5-hour used".
  readonly label: string;
  /// One value per bucket. Null is no reading: left off the line and "—" in the tip, never 0
  /// (OV-C61).
  readonly values: readonly (number | null)[];
  /// A line's colour. Bars are always on the token ramp.
  readonly colour: ArToken | `--cds-${string}`;
}

const lastOf = (values: readonly (number | null)[]): number => {
  for (let i = values.length - 1; i >= 0; i--) if (values[i] !== null && values[i] !== undefined) return i;
  return -1;
};

/// A strip of buckets over time, one bar per bucket or one line per series, marking the last.
///
/// Bars (a tile's sparkline): one per bucket scaled to the largest, any bucket above zero at least
/// a pixel tall and an empty one blank, over a baseline, the bucket still filling in the darker
/// step (OV-C194). Lines (the quota history): a line per series over the same buckets, with rules
/// where asked, the latest point of each marked, a gap where a bucket has no reading (OV-C60,
/// OV-C61).
///
/// Every figure it draws is reachable without a hover: its accessible name says the span, the
/// latest and the peak; the arrow keys walk the buckets from the latest, each with its time and
/// figures; a tap pins a bucket's; and the lines' keys and ends are text.
export function Sparkline({
  label,
  buckets,
  series,
  kind = 'bars',
  max,
  rules = [],
  format = (v: number) => v.toLocaleString(),
  ends = false,
  legend,
  className,
}: {
  /// What the strip shows: "Prompt tokens per hour".
  label: string;
  /// Each bucket's time, in the formatters' words; their number is the strip's width in buckets.
  buckets: readonly string[];
  /// Bars draw the first series; lines draw them all.
  series: readonly Series[];
  kind?: 'bars' | 'lines';
  /// The top of the scale, 100 for percentages; the largest value when absent.
  max?: number;
  /// Lines only: values to rule across the strip, as 100, 90 and 0 are for a quota.
  rules?: readonly number[];
  /// A value in words.
  format?: (value: number) => string;
  /// Show the first and last buckets' times under the strip.
  ends?: boolean;
  /// What the strip measures, after the series' keys: "percent used, over the window charted below".
  legend?: string;
  className?: string;
}) {
  const plot = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState<string | null>(null);
  const n = buckets.length;
  const drawn = kind === 'bars' ? series.slice(0, 1) : series;
  // Never divides by zero: an empty strip is scaled to one.
  const top = max ?? Math.max(1, ...drawn.flatMap((s) => s.values.map((v) => v ?? 0)));
  const y = (v: number) => 100 - (100 * Math.min(Math.max(v, 0), top)) / top;
  const keys = buckets.map((_, i) => String(i));

  const said = drawn.map((s) => {
    const last = lastOf(s.values);
    const values = s.values.filter((v): v is number => v !== null && v !== undefined);
    const latest = last < 0 ? '—' : format(s.values[last]!);
    const peak = values.length ? format(Math.max(...values)) : '—';
    return `${s.label}: latest ${latest}, peak ${peak}`;
  });
  const span = n === 0 ? 'nothing yet' : n === 1 ? `1 bucket, ${buckets[0]}` : `${n} buckets from ${buckets[0]} to ${buckets[n - 1]}`;
  const named = `${label}, ${span}; ${said.join('; ')}`;

  const locate = (x: number): string | null => {
    const r = plot.current?.getBoundingClientRect();
    if (!r || !(r.width > 0) || n === 0) return null;
    return String(Math.min(n - 1, Math.max(0, Math.floor(((x - r.left) / r.width) * n))));
  };

  const tip = (key: string | null): Tip | null => {
    if (key === null) return null;
    const i = Number(key);
    return {
      title: buckets[i],
      rows: drawn.map((s) => {
        const v = s.values[i];
        return {
          value: v === null || v === undefined ? '—' : format(v),
          label: s.label,
          colour: kind === 'bars' ? (i === n - 1 ? '--ar-tok-3' : '--ar-tok') : s.colour,
          mark: kind === 'lines' ? 'line' : 'square',
        };
      }),
    };
  };

  const path = (values: readonly (number | null)[]) => {
    let d = '';
    let pen = false;
    values.forEach((v, i) => {
      if (v === null || v === undefined) {
        pen = false;
        return;
      }
      d += `${pen ? 'L' : 'M'}${(i + 0.5).toFixed(2)},${y(v).toFixed(2)}`;
      pen = true;
    });
    return d;
  };

  const width = Math.max(n, 1);
  return (
    <figure className={[styles.spark, className].filter(Boolean).join(' ')}>
      <DataTip label={named} keys={keys} start={keys[n - 1]} locate={locate} onActive={setActive} tip={tip}>
        <div ref={plot} className={styles.plot} data-kind={kind}>
          <svg className={styles.svg} viewBox={`0 0 ${width} 100`} preserveAspectRatio="none" aria-hidden="true" focusable="false">
            {kind === 'lines' && rules.map((r) => <line key={r} className={styles.rule} x1={0} x2={width} y1={y(r)} y2={y(r)} />)}
            {kind === 'bars' && <line className={styles.baseline} x1={0} x2={width} y1={100} y2={100} />}
            {kind === 'bars' &&
              drawn[0]?.values.map((v, i) => {
                if (v === null || v === undefined || !(v > 0)) return null;
                const h = Math.max(PIXEL, (100 * Math.min(v, top)) / top);
                return <rect key={i} className={styles.bar} data-bucket={i} data-last={i === n - 1 ? '' : undefined} x={i + 0.1} width={0.8} y={100 - h} height={h} />;
              })}
            {kind === 'lines' && drawn.map((s) => <path key={s.key} className={styles.line} data-series={s.key} d={path(s.values)} style={{ stroke: `var(${s.colour})` }} />)}
            {active !== null && <line className={styles.cross} x1={Number(active) + 0.5} x2={Number(active) + 0.5} y1={0} y2={100} />}
          </svg>
          {kind === 'lines' &&
            drawn.map((s) => {
              const last = lastOf(s.values);
              if (last < 0) return null;
              return (
                <span
                  key={s.key}
                  className={styles.mark}
                  data-series={s.key}
                  style={{ insetInlineStart: `${((last + 0.5) / width) * 100}%`, insetBlockStart: `${y(s.values[last]!)}%`, background: `var(${s.colour})` }}
                />
              );
            })}
        </div>
      </DataTip>
      {ends && n > 0 && (
        <div className={styles.ends}>
          <span>{buckets[0]}</span>
          <span>{buckets[n - 1]}</span>
        </div>
      )}
      {(kind === 'lines' || legend) && (
        <ul className={styles.legend}>
          {kind === 'lines' &&
            drawn.map((s) => (
              <li key={s.key} className={styles.key}>
                <span className={styles.swatch} style={{ background: `var(${s.colour})` }} aria-hidden="true" />
                {s.label}
              </li>
            ))}
          {legend && <li>{legend}</li>}
        </ul>
      )}
    </figure>
  );
}
