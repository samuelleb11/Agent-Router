// New for agent-router: one request as a card that opens in place into its detail. Custom because
// the list holds 300 of them, which rules out Carbon's ExpandableTile and the ResizeObserver it runs
// for each (docs/web-console-carbon.md, "Why Carbon, and what it does not cover"). Drawn from the
// dashboard's `renderRequests` and `renderDetail` (assets/index.html).
import { ChevronRight, Fork, Time, WarningAlt } from '@carbon/icons-react';
import { useId, type ReactNode } from 'react';
import { CONCEPTS, FAMILY_COLOUR, paint, type Family } from '~/lib/concepts';
import type { Provider } from '~/model/settings';
import { ProviderMark } from './ProviderMark';
import { ShareBar, type Part } from './ShareBar';
import styles from './RequestRow.module.scss';

/// One fact in the detail: its label and its value.
export interface Fact {
  readonly label: string;
  readonly value: ReactNode;
  /// An id or a hash, set in monospace.
  readonly mono?: boolean;
  /// Takes the detail's whole width.
  readonly wide?: boolean;
}

/// What the detail holds beyond the summary: built only while the row is open, since the list holds
/// up to 300 rows.
export interface Detail {
  readonly facts: readonly Fact[];
  /// The error the request ended in, at full width with its line breaks (OV-C158).
  readonly error?: string;
  /// The request's context window, part by part (OV-C158).
  readonly context?: readonly Part[];
}

/// A request at a glance, and everything recorded about it one press away.
///
/// The summary is a button that opens the detail in place (`aria-expanded`), and the detail is a
/// region named by it. Whether it is open is the caller's — the address holds the open rows, so they
/// stay open across polls and reloads (OV-C157). On a phone it is a two-line card, the first token
/// moving into the detail (OV-C159). Its words are on the card, never in a title: the badges say
/// "subagent" and "slow", and the detail gives the full time and model id (OV-C156). Given the
/// provider that served it, the card wears its mark alone and the detail names it, with the account
/// that served it, in words (OV-C231).
export function RequestRow({
  time,
  at,
  provider,
  account,
  model,
  modelShort,
  family,
  subagent = false,
  slow = false,
  status,
  error = false,
  prompt,
  max,
  figures,
  open,
  onToggle,
  detail,
  className,
}: {
  /// When it started: with seconds today, with the date otherwise.
  time: string;
  /// The full date and time, in the detail.
  at: string;
  /// The provider that served it: its mark alone on the card, its mark and name in the detail.
  provider?: Provider;
  /// The account that served it, in words, in the detail: "work@example.test · work".
  account?: string;
  /// The model's full id, in the detail.
  model: string | null;
  /// Its short name on the card, as every client spells it: "opus-4-5-20251101" (OV-C184).
  modelShort: string;
  family: Family;
  subagent?: boolean;
  /// Its first token was slower than the scope's p95.
  slow?: boolean;
  /// The HTTP status, when one was recorded.
  status: number | null;
  /// A status of 400 or more, or an error recorded (OV-C149).
  error?: boolean;
  /// Prompt tokens, for the bar; null when not recorded.
  prompt: number | null;
  /// The largest prompt in the filtered set, which the bar is drawn against.
  max: number;
  /// The card's figures, in the formatters' words: prompt "1.2k", cached "92% cached" or "—",
  /// output "1.1k out", cost "$0.012", first token "812 ms".
  figures: { readonly prompt: string; readonly cached: string; readonly output: string; readonly cost: string; readonly firstToken: string };
  open: boolean;
  onToggle: () => void;
  detail?: () => Detail;
  className?: string;
}) {
  const ids = useId();
  const button = `${ids}-summary`;
  const region = `${ids}-detail`;
  const share = prompt !== null && max > 0 ? Math.min(100, (100 * Math.max(0, prompt)) / max) : null;
  const more = open && detail ? detail() : null;
  // The time and the model in full, and the first token, which a phone's card leaves to the detail.
  const facts: readonly Fact[] = open
    ? [
        { label: 'at', value: at },
        { label: 'model', value: model ?? '—', mono: true },
        ...(provider !== undefined ? [{ label: 'provider', value: <ProviderMark provider={provider} /> }] : []),
        ...(account !== undefined ? [{ label: 'account', value: account }] : []),
        { label: 'first token', value: figures.firstToken },
        ...(more?.facts ?? []),
      ]
    : [];

  return (
    <div className={[styles.row, className].filter(Boolean).join(' ')}>
      <button
        id={button}
        type="button"
        className={styles.summary}
        aria-expanded={open}
        aria-controls={open ? region : undefined}
        onClick={onToggle}
      >
        <ChevronRight className={styles.chev} aria-hidden="true" />
        {/* Spaces between the cells, so the button's name reads as words. */}
        <span className={styles.head}>
          <span className={styles.time}>{time}</span>{' '}
          {provider !== undefined && (
            <>
              <ProviderMark provider={provider} alone />{' '}
            </>
          )}
          <span className={styles.model}>
            <span className={styles.swatch} data-family={family} style={family === 'other' ? undefined : { background: paint(FAMILY_COLOUR[family]) }} aria-hidden="true" />
            <span className={styles.modelName}>{modelShort}</span>
          </span>{' '}
          <span className={styles.badges}>
            {subagent && (
              <span className={styles.badge}>
                <Fork aria-hidden="true" style={{ fill: paint(CONCEPTS.subagents.colour) }} />
                subagent
              </span>
            )}{' '}
            {slow && (
              <span className={styles.badge}>
                <Time aria-hidden="true" style={{ fill: paint(CONCEPTS.slow.colour) }} />
                slow
              </span>
            )}
          </span>
        </span>{' '}
        <span className={styles.status}>
          {error ? (
            <span className={styles.error}>
              <WarningAlt aria-hidden="true" style={{ fill: paint(CONCEPTS.errors.colour) }} />
              {status !== null ? String(status) : 'ERR'}
            </span>
          ) : status !== null ? (
            String(status)
          ) : (
            '—'
          )}
        </span>{' '}
        <span className={styles.bar} aria-hidden="true">
          {share !== null && share > 0 && <span className={styles.fill} style={{ inlineSize: `${share}%` }} />}
        </span>
        <span className={styles.prompt}>{figures.prompt}</span> <span className={styles.cached}>{figures.cached}</span>{' '}
        <span className={styles.out}>
          {figures.output} · {figures.cost}
        </span>{' '}
        <span className={styles.ttfb}>{figures.firstToken} first token</span>
      </button>
      {open && (
        <div id={region} role="region" aria-labelledby={button} className={styles.detail}>
          <dl className={styles.facts}>
            {facts.map((f, i) => (
              <div key={`${i}-${f.label}`} className={styles.fact} data-wide={f.wide ? '' : undefined}>
                <dt>{f.label}</dt>
                <dd data-mono={f.mono ? '' : undefined}>{f.value}</dd>
              </div>
            ))}
          </dl>
          {more?.error && <p className={styles.message}>{more.error}</p>}
          {more?.context && more.context.length > 0 && <ShareBar label="Context window" parts={more.context} unit="est. tokens" of="of this window" size="sm" />}
        </div>
      )}
    </div>
  );
}
