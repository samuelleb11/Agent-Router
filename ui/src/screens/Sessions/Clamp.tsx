// New for agent-router: the live view's `clampable` (assets/live.html) as a component. Anything tall
// folds behind a toggle and is never cut (SE-C85).
import { useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import styles from './Feed.module.scss';

/// How far a box may overflow its fold and still count as fitting.
const SLACK_PX = 4;

type Fold = { readonly at: 'measuring' } | { readonly at: 'fits' } | { readonly at: 'folded' | 'open'; readonly lines: number };

/// The line height a fold counts lines in: its first line of code or text, else its own.
function lineHeight(el: HTMLElement): number {
  const line = el.querySelector('pre, code, p') ?? el;
  const h = parseFloat(getComputedStyle(line).lineHeight);
  return Number.isFinite(h) && h > 0 ? h : 18;
}

/// A fold that clamps first and measures second (SE-C86): an unclamped box is always exactly as
/// tall as its content, so the page that measured first never folded anything (b124c9a). It is
/// drawn clamped, measured before it is painted, and unclamped when its content fits within four
/// pixels; otherwise it keeps the fold and a toggle that says how many lines it hides, "Show
/// 42 lines", which becomes "Show less" (SE-C85). `kind` sets the fold: prose and plain rows at
/// 11em, terminal output at four lines.
export function Clamp({ kind, children, className }: { kind: 'prose' | 'term'; children: ReactNode; className?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const id = useId();
  const [fold, setFold] = useState<Fold>({ at: 'measuring' });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    // The content's own height against the fold's, both read with the clamp on. Not the box's
    // scroll height, which also counts what is laid out of the flow, such as a snippet's copy
    // button hanging off its corner.
    const content = el.firstElementChild;
    const natural = content ? content.getBoundingClientRect().height : el.scrollHeight;
    const shown = el.getBoundingClientRect().height;
    if (natural <= shown + SLACK_PX) setFold({ at: 'fits' });
    else setFold({ at: 'folded', lines: Math.max(1, Math.round(natural / lineHeight(el))) });
  }, []);
  const clamped = fold.at === 'measuring' || fold.at === 'folded';
  return (
    <div className={[styles.clampWrap, className].filter(Boolean).join(' ')}>
      <div
        ref={box}
        id={id}
        className={clamped ? `${styles.clamp} ${kind === 'term' ? styles.clampTerm : styles.clampProse}` : undefined}
        data-fold={fold.at}
      >
        {children}
      </div>
      {(fold.at === 'folded' || fold.at === 'open') && (
        <button
          type="button"
          className={styles.more}
          aria-expanded={fold.at === 'open'}
          aria-controls={id}
          onClick={() => setFold({ at: fold.at === 'open' ? 'folded' : 'open', lines: fold.lines })}
        >
          {fold.at === 'open' ? 'Show less' : `Show ${fold.lines} lines`}
        </button>
      )}
    </div>
  );
}
