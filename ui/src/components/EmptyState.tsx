// Copied from Northstar ui/src/components/EmptyState.tsx at e9c1589.
// Changed: nothing. Its docs/13 is Northstar's console contracts, which bind here too (docs/web-console-carbon.md).
import type { ReactNode } from 'react';
import styles from './EmptyState.module.scss';

/// An empty list says WHICH empty it is. `docs/13-console-contracts.md` §8.
///
/// "Nothing filed" and "filters excluded everything" are different facts with different
/// next steps, and "no items" for both sends half the readers to the wrong one.
export function EmptyState({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className={styles.empty}>
      <p className={styles.title}>{title}</p>
      {children && <p className={styles.body}>{children}</p>}
      {action && <div className={styles.action}>{action}</div>}
    </div>
  );
}
