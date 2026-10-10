// New for agent-router: a tooltip for data that a tap can pin, so the facts in a tooltip reach a
// phone and a keyboard (docs/web-console-carbon.md, "Why Carbon, and what it does not cover").
// Drawn from the dashboard's `bindTip`, `showTip` and `placeTip` (assets/index.html), whose tips
// followed the pointer only, so a tap at most flashed them and the keyboard never reached them.
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import type { ArToken } from '~/lib/concepts';
import styles from './DataTip.module.scss';

/// SH-C31's geometry, in CSS pixels because it is measured against the pointer: a tip opens 12 px
/// right of the point and 14 px below it, flips 12 px to its other side rather than cross the
/// window's edge, and never comes nearer than 8 px to one.
const RIGHT = 12;
const BELOW = 14;
const FLIP = 12;
const EDGE = 8;

/// One figure in a tip: the value with its label, and the key of the series it belongs to.
export interface TipRow {
  readonly value: string;
  readonly label: string;
  /// The series' colour. A row with none keeps its alignment (SH-C34).
  readonly colour?: ArToken | `--cds-${string}`;
  /// How the series is drawn: a square for a bar, a line for a line (SH-C34).
  readonly mark?: 'square' | 'line';
}

export interface Tip {
  readonly title?: string;
  readonly rows?: readonly (TipRow | null | false | undefined)[];
}

type Mode = 'hover' | 'focus' | 'pinned';

interface Open<K> {
  readonly mode: Mode;
  /// The part pointed at, for a target with parts; null for a target that is one thing.
  readonly active: K | null;
  /// The pointer, while it hovers; null when the tip is anchored under the target.
  readonly at: { readonly x: number; readonly y: number } | null;
}

/// Where a tip `w` wide and `h` tall goes for the point (x, y).
function place(x: number, y: number, w: number, h: number): { left: number; top: number } {
  let left = x + RIGHT;
  let top = y + BELOW;
  if (left + w > window.innerWidth - EDGE) left = x - w - FLIP;
  if (top + h > window.innerHeight - EDGE) top = y - h - FLIP;
  return { left: Math.max(EDGE, left), top: Math.max(EDGE, top) };
}

/// A target and the tip that explains it.
///
/// - A mouse or a pen hovering shows the tip at the pointer, following it; leaving closes it.
/// - Focus shows the same tip anchored under the target (SH-C33), and so does a tap, which pins
///   it: it stays until the target is tapped again, a tap lands elsewhere, Escape is pressed or
///   the page scrolls. A click pins on a desk too. Enter and Space pin from the keyboard.
/// - A target with parts (a share bar's segments, a strip's buckets) says which part is under a
///   point through `locate`, and lists its parts in `keys` for the arrow keys to step through;
///   `onActive` tells it which part to light.
///
/// The tip is built by `tip` each time it draws, so it always shows the latest figures (SH-C32),
/// and it takes no pointer events. Nothing should live only in a tip: what it says is also on the
/// page as text (SH-C30); the tip is the way to it in place.
export function DataTip<K extends string = string>({
  label,
  role = 'img',
  tip,
  locate,
  keys,
  start,
  onActive,
  children,
  className,
}: {
  /// The target's accessible name: what it is, in words.
  label: string;
  role?: 'img' | 'group';
  /// The tip for a part, or for the whole target when `active` is null. Null shows nothing.
  tip: (active: K | null) => Tip | null;
  /// The part under the point (x, y), in client coordinates, or null for none.
  locate?: (x: number, y: number) => K | null;
  /// The parts in order, for the arrow keys.
  keys?: readonly K[];
  /// The part focus opens on; the first of `keys` when absent. A strip over time opens on its
  /// latest bucket.
  start?: K;
  /// The part the tip is about now, or null once it closes.
  onActive?: (active: K | null) => void;
  children: ReactNode;
  className?: string;
}) {
  const id = useId();
  const target = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState<Open<K> | null>(null);
  const isOpen = open !== null;
  const pinned = open?.mode === 'pinned';

  // Tell the owner what to light, once per change, through a ref so a new callback each render
  // is not a change.
  const onActiveRef = useRef(onActive);
  useLayoutEffect(() => {
    onActiveRef.current = onActive;
  });
  const active = open ? open.active : null;
  useEffect(() => {
    onActiveRef.current?.(active);
  }, [active]);

  // Any scroll closes it, pinned or not: a tip left where the target was is pointing at nothing.
  useEffect(() => {
    if (!isOpen) return;
    const scrolled = () => setOpen(null);
    window.addEventListener('scroll', scrolled, true);
    return () => window.removeEventListener('scroll', scrolled, true);
  }, [isOpen]);

  // A pinned tip stays through a leave and a blur, and goes on a press elsewhere or Escape.
  useEffect(() => {
    if (!pinned) return;
    const away = (e: Event) => {
      if (!target.current?.contains(e.target as Node)) setOpen(null);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(null);
    };
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', escape);
    };
  }, [pinned]);

  // Placed after it is drawn, when its size is known, and again whenever it redraws.
  useLayoutEffect(() => {
    const el = box.current;
    const t = target.current;
    if (!open || !el || !t) return;
    const anchor = open.at ?? { x: t.getBoundingClientRect().left, y: t.getBoundingClientRect().bottom };
    const size = el.getBoundingClientRect();
    const { left, top } = place(anchor.x, anchor.y, size.width, size.height);
    el.style.left = `${left}px`;
    el.style.top = `${top}px`;
  });

  const first = (): K | null => start ?? keys?.[0] ?? null;

  const point = (e: ReactPointerEvent<HTMLDivElement>) => {
    // A finger has no hover: a tap pins instead.
    if (e.pointerType === 'touch') return;
    const part = locate ? locate(e.clientX, e.clientY) : null;
    const at = { x: e.clientX, y: e.clientY };
    setOpen((now) => (now?.mode === 'pinned' ? now : { mode: 'hover', active: part, at }));
  };

  const leave = () => setOpen((now) => (now?.mode === 'hover' ? null : now));

  const focus = () => setOpen((now) => now ?? { mode: 'focus', active: first(), at: null });

  const blur = () => setOpen((now) => (now?.mode === 'focus' ? null : now));

  /// Pin the tip on `part`, or unpin it when it is already pinned there.
  const pin = (part: K | null) =>
    setOpen((now) => (now?.mode === 'pinned' && now.active === part ? null : { mode: 'pinned', active: part, at: null }));

  const tap = (e: ReactMouseEvent<HTMLDivElement>) => pin(locate ? locate(e.clientX, e.clientY) : (open?.active ?? first()));

  const press = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape') {
      if (open) {
        // The tip goes first; whatever holds the target closes on the next Escape.
        e.stopPropagation();
        setOpen(null);
      }
      return;
    }
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      pin(open?.active ?? first());
      return;
    }
    if (!keys || keys.length === 0) return;
    const step = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    const jump = e.key === 'Home' ? 0 : e.key === 'End' ? keys.length - 1 : null;
    if (step === 0 && jump === null) return;
    e.preventDefault();
    const at = open?.active != null ? keys.indexOf(open.active) : -1;
    const index = jump ?? (at < 0 ? (step > 0 ? 0 : keys.length - 1) : (at + step + keys.length) % keys.length);
    const part = keys[index] ?? null;
    setOpen((now) => ({ mode: now?.mode === 'pinned' ? 'pinned' : 'focus', active: part, at: null }));
  };

  const content = open ? tip(open.active) : null;
  const rows = (content?.rows ?? []).filter((r): r is TipRow => Boolean(r));
  const shown = content !== null && (Boolean(content.title) || rows.length > 0);

  return (
    <>
      <div
        ref={target}
        className={[styles.target, className].filter(Boolean).join(' ')}
        role={role}
        aria-label={label}
        aria-describedby={shown ? id : undefined}
        tabIndex={0}
        data-tip={open?.mode}
        onPointerEnter={point}
        onPointerMove={point}
        onPointerLeave={leave}
        onFocus={focus}
        onBlur={blur}
        onClick={tap}
        onKeyDown={press}
      >
        {children}
      </div>
      {shown &&
        createPortal(
          <div ref={box} id={id} role="tooltip" className={styles.tip} data-mode={open?.mode}>
            {content.title && <div className={styles.title}>{content.title}</div>}
            {rows.map((row, i) => (
              <div key={i} className={styles.row}>
                <span
                  className={styles.key}
                  data-mark={row.colour ? (row.mark ?? 'square') : 'none'}
                  style={row.colour ? { background: `var(${row.colour})` } : undefined}
                  aria-hidden="true"
                />
                <span className={styles.value}>{row.value}</span> <span className={styles.label}>{row.label}</span>
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
