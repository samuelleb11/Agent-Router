// New for agent-router: a list of bar rows. Custom because its rows are selectable `aria-pressed`
// buttons that scope the page, with a meta line and a subagent overlay, where Carbon's
// ContainedListItem puts extra props on its <li>, not its button (docs/web-console-carbon.md, "Why
// Carbon, and what it does not cover"). Drawn from the dashboard's `makeBarRow`, `setBarRow` and
// `fitTip` (assets/index.html).
import { WarningAlt } from '@carbon/icons-react';
import { useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { CONCEPTS, paint, type ArToken, type Concept } from '~/lib/concepts';
import styles from './BarList.module.scss';

/// How far the figure sits from the bar's end, in CSS pixels, for measuring: $spacing-02 at
/// Carbon's 16 px root, which `--gap` gives the stylesheet.
const GAP = 4;

export interface BarRow {
  /// Who the row is from one poll to the next — a project by its path, a session by its id — so
  /// focus and hover stay on it (OV-C170).
  readonly key: string;
  readonly name: string;
  /// A session's short id is set in monospace.
  readonly mono?: boolean;
  /// Its concept's icon, in the concept's colour (OV-C4).
  readonly concept?: Concept;
  /// Or a mark of its own, such as a model family's swatch.
  readonly mark?: ReactNode;
  /// A project's working directory, shown whole.
  readonly path?: string;
  /// Branch, project, client and model chips.
  readonly chips?: ReactNode;
  /// "N sessions · N req · out · % cached · cost · last seen", in the formatters' words.
  readonly meta?: string;
  /// "3 errors": drawn with the error mark, in the error colour (OV-C5).
  readonly errors?: string;
  /// The magnitude the bar draws. Null draws no bar; zero, a faint stub.
  readonly value: number | null;
  /// The part of `value` from subagents, laid over the bar's end (OV-C192).
  readonly sub?: number | null;
  /// The figure at the bar's end: "1.20M".
  readonly figure?: string;
  /// What the overlay is: "34% subagents", "12 by subagents".
  readonly subFigure?: string;
  /// The bar's colour; the token ramp's slate when absent, since a bar is a magnitude.
  readonly colour?: ArToken;
}

type Place = 'after' | 'inside' | 'below';

/// Rows of a label line over a bar scaled to the largest row: never a table, never wider than a
/// phone (OV-C129).
///
/// Given `onSelect`, each row is a button pressed while it is the scope; pressing it again is the
/// caller's to read as "clear" (OV-C23). Every word a row says is on it, at every width: the path
/// whole, the meta line wrapping on a phone, the figure at the bar. Nothing waits for a hover.
export function BarList({
  label,
  rows,
  selected = null,
  onSelect,
  className,
}: {
  /// What the list is, as its accessible name: "Projects".
  label: string;
  rows: readonly BarRow[];
  /// The key of the row the page is scoped to.
  selected?: string | null;
  onSelect?: (key: string) => void;
  className?: string;
}) {
  const max = rows.reduce((m, r) => Math.max(m, r.value ?? 0), 0);
  const choosing = onSelect !== undefined && selected !== null && rows.some((r) => r.key === selected);
  return (
    <ul className={[styles.list, className].filter(Boolean).join(' ')} aria-label={label} data-selection={choosing ? '' : undefined}>
      {rows.map((row) => (
        <li key={row.key} className={styles.item}>
          <Row row={row} max={max} pressed={onSelect ? selected === row.key : undefined} onSelect={onSelect} />
        </li>
      ))}
    </ul>
  );
}

function Row({ row, max, pressed, onSelect }: { row: BarRow; max: number; pressed: boolean | undefined; onSelect?: (key: string) => void }) {
  const ids = useId();
  const id = (part: string) => `${ids}-${part}`;
  const track = useRef<HTMLSpanElement>(null);
  const value = useRef<HTMLSpanElement>(null);
  const [place, setPlace] = useState<Place>('after');

  const v = row.value;
  // Scaled to the largest without dividing by zero (OV-C191).
  const pct = v !== null && max > 0 ? (100 * Math.max(0, v)) / max : 0;
  const sub = Math.min(Math.max(0, row.sub ?? 0), Math.max(0, v ?? 0));
  const figure = row.figure ?? (v === null ? '—' : v.toLocaleString());

  // The figure goes past the bar's end when there is room, inside the bar when it fits there, and
  // on a line of its own otherwise; measured, because its length varies by row, and again
  // whenever the track changes width (OV-C193, OV-C172).
  useLayoutEffect(() => {
    const t = track.current;
    const fit = () => {
      const width = t?.getBoundingClientRect().width ?? 0;
      const own = value.current?.getBoundingClientRect().width ?? 0;
      if (!width) return;
      const bar = (width * pct) / 100;
      setPlace(bar + GAP + own <= width ? 'after' : own + 2 * GAP <= bar ? 'inside' : 'below');
    };
    fit();
    if (!t) return;
    const watch = new ResizeObserver(fit);
    watch.observe(t);
    return () => watch.disconnect();
  }, [pct, figure, row.subFigure]);

  const Icon = row.concept ? CONCEPTS[row.concept].icon : null;
  const at = place === 'after' ? { insetInlineStart: `calc(${pct}% + var(--gap))` } : place === 'inside' ? { insetInlineEnd: `calc(${100 - pct}% + var(--gap))` } : undefined;
  const described = [row.path && id('path'), row.chips && id('chips'), (row.meta || row.errors) && id('meta'), id('value')].filter(Boolean).join(' ');

  const body = (
    <>
      <span className={styles.line}>
        {(row.mark || Icon) && (
          <span className={styles.mark} aria-hidden="true">
            {row.mark ?? (Icon && row.concept && <Icon style={{ fill: paint(CONCEPTS[row.concept].colour) }} />)}
          </span>
        )}
        <span id={id('name')} className={styles.name} data-mono={row.mono ? '' : undefined}>
          {row.name}
        </span>
        {row.path && (
          <span id={id('path')} className={styles.path}>
            {row.path}
          </span>
        )}
        {row.chips && (
          <span id={id('chips')} className={styles.chips}>
            {row.chips}
          </span>
        )}
        {(row.meta || row.errors) && (
          <span id={id('meta')} className={styles.meta}>
            {row.meta}
            {row.meta && row.errors && ' · '}
            {row.errors && (
              <span className={styles.errors}>
                <WarningAlt style={{ fill: paint(CONCEPTS.errors.colour) }} aria-hidden="true" />
                {row.errors}
              </span>
            )}
          </span>
        )}
      </span>
      <span ref={track} className={styles.track}>
        <span className={styles.rail}>
          {v !== null && (
            <span className={styles.bar} data-zero={v === 0 ? '' : undefined} style={{ inlineSize: `${pct}%` }}>
              <span className={styles.main} style={{ flexGrow: Math.max(0, v - sub), background: paint(row.colour ?? '--ar-tok') }} />
              {sub > 0 && <span className={styles.sub} style={{ flexGrow: sub }} />}
            </span>
          )}
        </span>
        <span ref={value} id={id('value')} className={styles.value} data-place={place} style={at}>
          {figure}
          {row.subFigure && (
            <>
              {' '}
              <span className={styles.subLabel}>{row.subFigure}</span>
            </>
          )}
        </span>
      </span>
    </>
  );

  if (!onSelect) return <div className={styles.row}>{body}</div>;
  return (
    <button
      type="button"
      className={styles.row}
      aria-pressed={pressed}
      aria-labelledby={id('name')}
      aria-describedby={described}
      onClick={() => onSelect(row.key)}
    >
      {body}
    </button>
  );
}
