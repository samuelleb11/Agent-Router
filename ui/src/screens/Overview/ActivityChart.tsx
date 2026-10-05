// New for agent-router: the activity chart, Carbon Charts' StackedBarChart in the model-family colours.
// The chart and its stylesheet are loaded only through import(), so they arrive in a chunk of their own
// (eslint.config.mjs, scripts/check-chunks.mjs) when there are bars to draw. Adapted from Northstar's
// StatusChart (ui/src/screens/Dashboard/StatusChart.tsx at d46713f): the look read off the tokens in
// force, a guard that keeps a throwing chart from taking the page down, and figures handed over only
// when one of them changes.
import { SkeletonPlaceholder } from '@carbon/react';
import { Component, lazy, Suspense, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { DataTip } from '~/components/DataTip';
import type { Timeline } from '~/data/overview';
import { useNarrow } from '~/lib/viewport';
import { bucketName } from '~/model/format';
import { bucketTip, chartData, chartOptions, errorMarks, FAMILIES, FAMILY_TOKENS, niceTop, timeLabels, type Bucket, type ChartLook } from './timeline';
import styles from './ActivityChart.module.scss';

/// Carbon Charts and their stylesheet, as Northstar's dashboard takes them, in one chunk of their own.
const StackedBarChart = lazy(async () => {
  const [charts] = await Promise.all([import('@carbon/charts-react'), import('@carbon/charts/styles.css')]);
  return { default: charts.StackedBarChart };
});

/// The family fills, read off the tokens `styles.scss` defines for the theme in force, and the
/// theme from the class on <html>, which is the truth about it: one theme state, shared (SH-C36).
function readLook(): ChartLook {
  const root = document.documentElement;
  const cs = getComputedStyle(root);
  const fills = Object.fromEntries(FAMILIES.map((f) => [f, cs.getPropertyValue(FAMILY_TOKENS[f]).trim()])) as Record<(typeof FAMILIES)[number], string>;
  return { theme: root.classList.contains('ar-dark') ? 'g100' : 'white', fills: FAMILIES.every((f) => fills[f] !== '') ? fills : null };
}

function useLook(): ChartLook {
  const [look, setLook] = useState(readLook);
  useEffect(() => {
    const observer = new MutationObserver(() => {
      const next = readLook();
      setLook((was) => (JSON.stringify(was) === JSON.stringify(next) ? was : next));
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
    return () => observer.disconnect();
  }, []);
  return look;
}

/// Whether the tab is on screen.
function useVisible(): boolean {
  const [visible, set] = useState(() => document.visibilityState !== 'hidden');
  useEffect(() => {
    const on = () => set(document.visibilityState !== 'hidden');
    on();
    document.addEventListener('visibilitychange', on);
    return () => document.removeEventListener('visibilitychange', on);
  }, []);
  return visible;
}

/// `next` while the tab is on screen; while it is hidden, what it was when the tab was last on
/// screen, so a hidden chart is handed nothing and, shown again, is handed the latest once (OV-C213).
function useKeptWhileHidden<T>(next: T): T {
  const visible = useVisible();
  const [kept, keep] = useState(next);
  if (visible && kept !== next) keep(next);
  return visible ? next : kept;
}

/// A chart is drawn from SVG measurements, and a library that throws while measuring must not take
/// the page down with it.
class Drawn extends Component<{ children: ReactNode }, { why: string | null }> {
  override state: { why: string | null } = { why: null };

  static getDerivedStateFromError(e: unknown): { why: string } {
    return { why: e instanceof Error ? e.message : String(e) };
  }

  override render() {
    return this.state.why === null ? (
      this.props.children
    ) : (
      <p className={styles.unavailable} role="status">
        The chart could not be drawn — {this.state.why}. Each bucket&rsquo;s figures are in its tip.
      </p>
    );
  }
}

interface Plot {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

const samePlot = (a: Plot | null, b: Plot | null) => !!a && !!b && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height;

/// The bars' area inside the chart, measured where Carbon Charts drew it, relative to `box`.
function measurePlot(box: HTMLElement): Plot | null {
  const grid = box.querySelector('.chart-grid-backdrop');
  if (!grid) return null;
  const outer = box.getBoundingClientRect();
  const r = grid.getBoundingClientRect();
  if (!(r.width > 0)) return null;
  return { left: Math.round(r.left - outer.left), top: Math.round(r.top - outer.top), width: Math.round(r.width), height: Math.round(r.height) };
}

export function ActivityChart({ bk, timeline, now }: { bk: readonly Bucket[]; timeline: Timeline; now: number }) {
  const look = useLook();
  const narrow = useNarrow();
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [plot, setPlot] = useState<Plot | null>(null);
  const [active, setActive] = useState<string | null>(null);

  // Drawn to the width it is shown, and drawn again when that changes (OV-C172).
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    let frame = 0;
    const measure = () => {
      const w = Math.round(el.getBoundingClientRect().width);
      setWidth((was) => (was === w ? was : w));
      const p = measurePlot(el);
      setPlot((was) => (samePlot(was, p) ? was : p));
    };
    const soon = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    measure();
    const resized = new ResizeObserver(soon);
    resized.observe(el);
    const drawn = new MutationObserver(soon);
    drawn.observe(el, { subtree: true, childList: true });
    return () => {
      cancelAnimationFrame(frame);
      resized.disconnect();
      drawn.disconnect();
    };
  }, []);

  const plotWidth = plot?.width ?? Math.max(0, width - 56);
  const labels = timeLabels(bk, timeline.bucket_ms, timeline, plotWidth, narrow);
  const name = `Prompt tokens per ${bucketName(timeline.bucket_ms)}, by model family`;

  // Carbon Charts redraws for any prop that is a new object, and a redraw starts a d3 transition
  // that a hidden tab never runs. So the chart is handed the same objects until a figure, a label or
  // the look changes (OV-C171), and nothing while the tab is hidden (OV-C213).
  const figures = JSON.stringify(chartData(bk));
  const data = useMemo(() => JSON.parse(figures) as ReturnType<typeof chartData>, [figures]);
  const shape = JSON.stringify({ keys: bk.map((b) => b.t), top: niceTop(Math.max(0, ...bk.map((b) => b.prompt))), labels, look, name });
  // Keyed by what the options say, not by the objects they were built from.
  const options = useMemo(() => chartOptions(bk, labels, look, name), [shape]);
  const props = useMemo(() => ({ data, options }), [data, options]);
  const handed = useKeptWhileHidden(props);

  const nB = bk.length;
  const keys = useMemo(() => bk.map((_, i) => String(i)), [bk]);
  const top = niceTop(Math.max(0, ...bk.map((b) => b.prompt)));
  const locate = (x: number): string | null => {
    const el = box.current;
    if (!el || !nB) return null;
    const p = measurePlot(el) ?? plot;
    const outer = el.getBoundingClientRect();
    const left = outer.left + (p?.left ?? 0);
    const w = p?.width ?? outer.width;
    if (!(w > 0)) return null;
    return String(Math.min(nB - 1, Math.max(0, Math.floor(((x - left) / w) * nB))));
  };
  const at = (i: number) => (plot ? plot.left + ((i + 0.5) * plot.width) / nB : null);
  const marks = errorMarks(bk);
  const activeIndex = active === null ? null : Number(active);
  const activeX = activeIndex === null ? null : at(activeIndex);

  return (
    <DataTip
      label={`${name}: ${nB} buckets; pick one for its figures`}
      keys={keys}
      start={keys[nB - 1]}
      locate={locate}
      onActive={setActive}
      tip={(key) => (key === null ? null : bucketTip(bk[Number(key)]!, timeline.bucket_ms, now))}
      className={styles.target}
    >
      <div ref={box} className={styles.chart} data-activity-chart="">
        <Drawn>
          <Suspense fallback={<SkeletonPlaceholder className={styles.loading} />}>
            <StackedBarChart data={handed.data} options={handed.options} />
          </Suspense>
        </Drawn>
        {plot && (
          <div className={styles.overlay} aria-hidden="true">
            {/* A bucket with errors carries a mark above its bar, in the error colour (OV-C115). */}
            {marks.map((m) => (
              <span
                key={m.index}
                className={styles.errorMark}
                data-error-mark={m.index}
                style={{ left: `${at(m.index)}px`, top: `${plot.top + plot.height * (1 - Math.min(1, m.total / top))}px` }}
              />
            ))}
            {activeX !== null && <span className={styles.cross} style={{ left: `${activeX}px`, top: `${plot.top}px`, height: `${plot.height}px` }} />}
          </div>
        )}
      </div>
    </DataTip>
  );
}
