// Copied from Northstar ui/src/components/StatTile.tsx at e9c1589.
// Changed: the tones are the router's status (ok, warn, bad) on its --ar-* tokens, not Northstar's buckets; a value that could not be read is an em dash with no warning tone, since on the Overview "—" is often a fact (no request priced, no latency yet) rather than a read that failed; and the tile takes children under its lines, for a sparkline, a meter or the action a figure offers.
import type { ReactNode } from 'react';
import { Tile } from '@carbon/react';
import styles from './StatTile.module.scss';

/// A tone is a MEANING, never decoration: ok, warn and bad are the router's status colours, and
/// everything else is neutral.
export type Tone = 'ok' | 'warn' | 'bad';

/// One stat tile: a label, a value, a detail line, what it means, and a tone stripe when the value
/// means a status.
///
/// The value is a node passed through as given. `null` and `undefined` are the one case this tile
/// decides for itself: they render an em dash, never a zero, because a figure the router did not
/// give is not a zero (SH-C20).
///
/// `meaning` is TEXT, at every width, never a title: a meaning only a hovering mouse can get is a
/// meaning a phone and a keyboard do not have (SH-C30).
export function StatTile({
  label,
  value,
  tone,
  detail,
  meaning,
  children,
  hero = false,
  className,
}: {
  label: string;
  /// A figure, a node, or `null`/`undefined` for a value the router did not give.
  value: ReactNode;
  tone?: Tone;
  /// One helper line under the value: what it was counted over.
  detail?: ReactNode;
  /// What the number means, in one sentence.
  meaning?: string;
  /// Under the lines: a sparkline, a meter, an action.
  children?: ReactNode;
  /// The largest tile of a row, which takes two columns on a phone.
  hero?: boolean;
  className?: string;
}) {
  const withheld = value === null || value === undefined;
  const cls = [styles.tile, tone ? styles[`tone_${tone}`] : undefined, hero ? styles.hero : undefined, className].filter(Boolean).join(' ');
  return (
    <Tile className={cls} data-tone={tone} data-withheld={withheld || undefined}>
      <span className={styles.label}>{label}</span>
      {withheld ? (
        <span className={styles.value} aria-label="not given">
          —
        </span>
      ) : (
        <span className={styles.value}>{value}</span>
      )}
      {detail !== undefined && detail !== null && detail !== '' && <span className={styles.detail}>{detail}</span>}
      {meaning !== undefined && <span className={styles.meaning}>{meaning}</span>}
      {children}
    </Tile>
  );
}
