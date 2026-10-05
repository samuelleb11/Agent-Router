// New for agent-router: a share bar and its legend. Custom because the row-to-segment highlight and
// the 24 px hover zone for thin segments are what the bar is read by, and Carbon's MeterChart loses
// both (docs/web-console-carbon.md, "Why Carbon, and what it does not cover"). Drawn from the
// dashboard's `setStack` and `legRow` (assets/index.html).
import { useRef, useState } from 'react';
import { CONCEPTS, paint, type ArToken, type Concept } from '~/lib/concepts';
import { DataTip, type Tip } from './DataTip';
import styles from './ShareBar.module.scss';

/// A part under this many CSS pixels owns a zone this wide around its centre, so a sliver is still
/// easy to point at (OV-C190). Geometry measured against the pointer, the same 24 px as
/// Carbon's $spacing-06.
const ZONE = 24;

export interface Part {
  /// Stable across polls, so a lit part stays lit through one (OV-C170).
  readonly key: string;
  /// The reader's word for it: "from cache", "main thread".
  readonly label: string;
  /// Its size. Null is not recorded, which the legend says as "—"; zero is left out of the bar.
  readonly value: number | null;
  readonly colour: ArToken;
  /// Its concept's icon in the legend, for the parts that are concepts (main thread, subagents,
  /// the four parts of the context window).
  readonly concept?: Concept;
  /// The value as the formatters write it, "1.20M". Its own digits when absent.
  readonly figure?: string;
  /// Anything else the part says, shown in its legend row and its tip: "5-min 1.2k · 1-hour 300".
  readonly detail?: string;
}

/// A share as the console writes one (OV-C179): "—" with nothing to divide by or nothing recorded,
/// whole from 10 % and at 0, one decimal below 10 %, so a small share is never "0 %".
function shareOf(value: number | null, total: number): string {
  if (value === null || !(total > 0)) return '—';
  const p = (100 * value) / total;
  return `${p >= 10 || p === 0 ? p.toFixed(0) : p.toFixed(1)}%`;
}

const drawn = (value: number | null): value is number => typeof value === 'number' && value > 0;

/// The parts of a whole, side by side, and a legend that names each with its figure and share.
///
/// Every fact is on the page without a hover: the legend says each part's figure, share and
/// detail, and the bar's accessible name says them all. Pointing at a part, or at its legend
/// row, lights it; pointing at a part, focusing the bar and walking it with the arrow keys, or
/// tapping it, gives its figures in a `DataTip`.
export function ShareBar({
  label,
  parts,
  total,
  unit = 'tokens',
  of = 'of the whole',
  size = 'md',
  legend = true,
  className,
}: {
  /// What the whole is: "What the model read".
  label: string;
  /// The parts, in the fixed order they are drawn (OV-C189).
  parts: readonly Part[];
  /// What a share is of. The parts' sum when absent.
  total?: number;
  /// What a figure counts, for the tip: "tokens", "est. tokens".
  unit?: string;
  /// What a share is of, for the tip: "of everything read".
  of?: string;
  size?: 'md' | 'sm';
  legend?: boolean;
  className?: string;
}) {
  const bar = useRef<HTMLDivElement>(null);
  const [hot, setHot] = useState<string | null>(null);
  const whole = total ?? parts.reduce((sum, p) => sum + (p.value ?? 0), 0);
  const shown = parts.filter((p) => drawn(p.value));
  const figure = (p: Part) => p.figure ?? (p.value === null ? '—' : p.value.toLocaleString());
  const named = `${label}: ${parts.map((p) => `${p.label} ${figure(p)}, ${shareOf(p.value, whole)}`).join('; ')}`;

  /// The part under the pointer. The parts are measured where they are drawn: a part under 24 px
  /// wins the 24 px zone around its centre over a wide neighbour.
  const locate = (x: number): string | null => {
    let hit: string | null = null;
    const boxes = [...(bar.current?.children ?? [])].map((el) => ({ key: (el as HTMLElement).dataset.key ?? null, r: el.getBoundingClientRect() }));
    for (const { key, r } of boxes) if (x >= r.left && x <= r.right) hit = key;
    for (const { key, r } of boxes) if (r.width < ZONE && Math.abs(x - (r.left + r.right) / 2) <= ZONE / 2) hit = key;
    return hit;
  };

  const tip = (key: string | null): Tip => {
    const p = parts.find((part) => part.key === key);
    if (!p) {
      return { title: label, rows: parts.map((part) => ({ value: figure(part), label: `${part.label} · ${shareOf(part.value, whole)}`, colour: part.colour })) };
    }
    return {
      title: p.label,
      rows: [
        { value: figure(p), label: unit, colour: p.colour },
        { value: shareOf(p.value, whole), label: of },
        p.detail ? { value: p.detail, label: '' } : null,
      ],
    };
  };

  return (
    <figure className={[styles.share, className].filter(Boolean).join(' ')}>
      <DataTip label={named} keys={shown.map((p) => p.key)} locate={locate} onActive={setHot} tip={tip}>
        <div ref={bar} className={styles.bar} data-size={size}>
          {shown.map((p) => (
            <span
              key={p.key}
              className={styles.segment}
              data-key={p.key}
              data-hot={hot === p.key ? '' : undefined}
              style={{ flexGrow: p.value ?? 0, background: paint(p.colour) }}
            />
          ))}
        </div>
      </DataTip>
      {legend && (
        <ul className={styles.legend} aria-label={`${label}, by part`}>
          {parts.map((p) => {
            const Icon = p.concept ? CONCEPTS[p.concept].icon : null;
            return (
              <li
                key={p.key}
                className={styles.item}
                data-key={p.key}
                data-hot={hot === p.key ? '' : undefined}
                onPointerEnter={() => setHot(p.key)}
                onPointerLeave={() => setHot((now) => (now === p.key ? null : now))}
              >
                {/* Spaces between the cells, so a row reads as words: "from cache 1.20M 80%". */}
                <span className={styles.swatch} style={{ background: paint(p.colour) }} aria-hidden="true" />
                <span className={styles.name}>
                  {Icon && p.concept && <Icon className={styles.icon} style={{ fill: paint(CONCEPTS[p.concept].colour) }} aria-hidden="true" />}
                  <span>{p.label}</span>
                  {p.detail && (
                    <>
                      {' '}
                      <span className={styles.detail}>{p.detail}</span>
                    </>
                  )}
                </span>{' '}
                <span className={styles.figure}>{figure(p)}</span> <span className={styles.pct}>{shareOf(p.value, whole)}</span>
              </li>
            );
          })}
        </ul>
      )}
    </figure>
  );
}
