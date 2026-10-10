// Copied from Northstar ui/src/components/StatGrid.tsx at e9c1589.
// Changed: nothing. Its docs/13 is Northstar's console contracts, which bind here too (docs/web-console-carbon.md).
import type { ReactNode } from 'react';
import styles from './StatGrid.module.scss';

/// The row of stat tiles. `auto-fit` with a minimum, so a row of seven tiles fills the
/// width evenly instead of leaving one orphan on a line of its own; two columns in a hand,
/// where a third would put a five-digit number on two lines.
///
/// A region of numbers says what they are about (`label`) and, through `describedBy`, which
/// read they came from: a count is only as true as its age (`docs/13` §6).
export function StatGrid({
  label,
  describedBy,
  children,
  className,
}: {
  /// What this row of numbers is about, as its accessible name.
  label?: string;
  /// The id of the element dating these numbers.
  describedBy?: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <section className={[styles.grid, className].filter(Boolean).join(' ')} aria-label={label} aria-describedby={describedBy}>
      {children}
    </section>
  );
}
