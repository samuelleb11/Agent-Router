// New for agent-router: a scope crumb and the picker it opens. Custom because its rows carry a name, a
// left-ellipsised path and a money meta, filterable, where Carbon's MenuItem labels are strings
// only (docs/web-console-carbon.md, "Why Carbon, and what it does not cover"). Built on Carbon's
// Popover and Search, with the listbox pattern rather than the dashboard's `role=menu` with an
// input inside it (`openMenu`, assets/index.html).
import { ChevronDown } from '@carbon/icons-react';
import { Popover, PopoverContent, Search } from '@carbon/react';
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { CONCEPTS, paint } from '~/lib/concepts';
import { useCoarse } from '~/lib/viewport';
import styles from './ScopePicker.module.scss';

/// More rows than this and the picker opens with a filter (OV-C18).
const FILTER_ABOVE = 8;

export interface ScopeOption {
  /// What choosing it sets the scope to.
  readonly key: string;
  readonly name: string;
  /// Drawn before the name: an account's provider's mark, a session's providers' (OV-C231, SE-C242).
  /// Words it carries are part of the row's name; the filter reads `name` and `sub` only.
  readonly mark?: ReactNode;
  /// A session's short id is set in monospace.
  readonly mono?: boolean;
  /// The second line: a project's path, a session's branch and start, an account's router id.
  readonly sub?: string;
  /// The second line is a path, and gives up its head rather than its tail.
  readonly path?: boolean;
  /// "1.20M · 840 req · $41.20", in the formatters' words.
  readonly meta?: string;
  /// The meta's tooltip: why a figure in it is marked, such as a cost's "+ 4 unpriced" (OV-C229,
  /// OV-C232); the picker's note says the same as text (SH-C30).
  readonly metaTip?: string;
}

export type Level = 'account' | 'project' | 'session';

const LEVEL_WORD: Readonly<Record<Level, string>> = { account: 'Account', project: 'Project', session: 'Session' };

/// One level of the scope as a crumb that opens its picker (OV-C14): a Popover holding a filter
/// above more than eight rows and a listbox of the rows, "All …" first as the caller gives them.
///
/// Opening it puts the focus in the filter, else on the current row, else on the first; Up and Down
/// move through the rows and wrap, Home and End go to the ends, Enter or Space chooses, and Escape
/// closes it and gives the focus back to the crumb. A press outside, a window resize, a second press
/// on the crumb or a scope set from elsewhere closes it (OV-C19, OV-C33). New rows from a poll
/// change it in place, never closing or moving it (OV-C15).
export function ScopePicker({
  level,
  crumb,
  set,
  value,
  options,
  note,
  onPick,
  onOpenChange,
  className,
}: {
  level: Level;
  /// What the crumb reads: "All projects", or the level's name as the lists write it (OV-C13).
  crumb: string;
  /// The level is set, so the crumb reads in the strong style (OV-C12).
  set: boolean;
  /// The key of the current choice.
  value: string;
  options: readonly ScopeOption[];
  /// What the list leaves out, said where it is read: "the 200 most recent sessions" (OV-C34).
  note?: string;
  onPick: (key: string) => void;
  /// Told when it opens and closes, so its rows can be read only while it is open (OV-C217).
  onOpenChange?: (open: boolean) => void;
  className?: string;
}) {
  const ids = useId();
  const listId = `${ids}-list`;
  const coarse = useCoarse();
  const trigger = useRef<HTMLButtonElement>(null);
  const filter = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [open, setOpen] = useState(false);
  const [term, setTerm] = useState('');
  const [active, setActive] = useState<string | null>(null);

  // A scope set from elsewhere — a list row, a link — closes the picker without moving the focus.
  const [seen, setSeen] = useState(value);
  if (seen !== value) {
    setSeen(value);
    setOpen(false);
    setTerm('');
    setActive(null);
  }

  const plural = `${level}s`;
  const filterable = options.length > FILTER_ABOVE;
  const words = term.trim().toLowerCase();
  const shown = words ? options.filter((o) => `${o.name} ${o.sub ?? ''}`.toLowerCase().includes(words)) : options;
  const Icon = CONCEPTS[level].icon;

  const close = (refocus: boolean) => {
    setOpen(false);
    setTerm('');
    setActive(null);
    if (refocus) trigger.current?.focus();
  };

  /// The row for `key`, found by its key rather than a selector built from it: a key is a path or
  /// an id, and either may hold anything.
  const rowOf = (key: string) => [...(list.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? [])].find((el) => el.dataset.key === key);

  useEffect(() => {
    onOpenChange?.(open);
  }, [open]);

  // Opened: the focus goes to the filter, else the current row, else the first. Only on opening, so
  // a poll's new rows never move it (OV-C15) — and once more when the rows it reads on opening bring
  // a filter with them (OV-C217).
  useEffect(() => {
    if (!open) return;
    if (filter.current) {
      filter.current.focus();
      return;
    }
    const start = options.find((o) => o.key === value) ?? options[0];
    if (start) {
      setActive(start.key);
      rowOf(start.key)?.focus();
    }
  }, [open, filterable]);

  // A window resize closes it: it was placed for the old window.
  useEffect(() => {
    if (!open) return;
    const resized = () => {
      setOpen(false);
      setTerm('');
      setActive(null);
    };
    window.addEventListener('resize', resized);
    return () => window.removeEventListener('resize', resized);
  }, [open]);

  const focusOption = (key: string | undefined) => {
    if (key === undefined) return;
    setActive(key);
    rowOf(key)?.focus();
  };

  const pick = (key: string) => {
    close(true);
    onPick(key);
  };

  const onOptionKey = (e: KeyboardEvent<HTMLLIElement>, key: string) => {
    const at = shown.findIndex((o) => o.key === key);
    const go = (i: number) => {
      e.preventDefault();
      focusOption(shown[(i + shown.length) % shown.length]?.key);
    };
    if (e.key === 'ArrowDown') go(at + 1);
    else if (e.key === 'ArrowUp') go(at - 1);
    else if (e.key === 'Home') go(0);
    else if (e.key === 'End') go(shown.length - 1);
    else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      pick(key);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      close(true);
    }
  };

  /// Carbon's Search swallows Escape to clear itself, and stops it there; preventing its default
  /// first keeps it from running, so Escape closes the picker as it does from a row.
  const onFilterKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      close(true);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      focusOption((shown.find((o) => o.key === value) ?? shown[0])?.key);
    } else if (e.key === 'Enter' && shown[0]) {
      e.preventDefault();
      pick(shown[0].key);
    }
  };

  const current = options.find((o) => o.key === value);
  const tabbable = active !== null && shown.some((o) => o.key === active) ? active : (current && shown.includes(current) ? current : shown[0])?.key;

  return (
    <Popover
      open={open}
      onRequestClose={() => close(false)}
      align="bottom-start"
      autoAlign
      caret={false}
      className={className}
    >
      <button
        ref={trigger}
        type="button"
        className={styles.crumb}
        data-set={set ? '' : undefined}
        aria-label={`${LEVEL_WORD[level]}: ${crumb}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        onClick={() => (open ? close(false) : setOpen(true))}
      >
        <Icon aria-hidden="true" style={{ fill: paint(CONCEPTS[level].colour) }} />
        <span className={styles.crumbText}>{crumb}</span>
        <ChevronDown aria-hidden="true" className={styles.caret} />
      </button>
      <PopoverContent>
        {open && (
          <div className={styles.panel}>
            {filterable && (
              <Search
                ref={filter}
                size={coarse ? 'lg' : 'sm'}
                labelText={`Filter ${plural}`}
                placeholder={`Filter ${plural}…`}
                closeButtonLabelText="Clear the filter"
                value={term}
                onChange={(e) => {
                  setTerm(e.target.value);
                  setActive(null);
                }}
                onKeyDown={onFilterKey}
                autoComplete="off"
                spellCheck={false}
              />
            )}
            {note && <p className={styles.note}>{note}</p>}
            {shown.length > 0 ? (
              <ul ref={list} id={listId} className={styles.list} role="listbox" aria-label={`${LEVEL_WORD[level]}s`}>
                {shown.map((o) => (
                  <li
                    key={o.key}
                    className={styles.option}
                    role="option"
                    aria-selected={o.key === value}
                    tabIndex={o.key === tabbable ? 0 : -1}
                    data-key={o.key}
                    onClick={() => pick(o.key)}
                    onKeyDown={(e) => onOptionKey(e, o.key)}
                    onFocus={() => setActive(o.key)}
                  >
                    {/* Spaces between the cells, so the row's name reads as words, not "agent-router/Users/…". */}
                    <span className={styles.name} data-mono={o.mono ? '' : undefined}>
                      {o.mark && (
                        <>
                          <span className={styles.mark}>{o.mark}</span>{' '}
                        </>
                      )}
                      {o.name}
                    </span>
                    {o.sub && (
                      <>
                        {' '}
                        <span className={styles.sub} data-path={o.path ? '' : undefined}>
                          {o.sub}
                        </span>
                      </>
                    )}
                    {o.meta && (
                      <>
                        {' '}
                        <span className={styles.meta} title={o.metaTip}>
                          {o.meta}
                        </span>
                      </>
                    )}
                  </li>
                ))}
              </ul>
            ) : (
              <p className={styles.empty} role="status">
                Nothing matches.
              </p>
            )}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
