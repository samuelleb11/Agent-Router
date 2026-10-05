// Copied from Northstar ui/src/components/Card.tsx at e9c1589.
// Changed: nothing. Its docs/13 is Northstar's console contracts, which bind here too (docs/web-console-carbon.md).
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { ClickableTile, Tile } from '@carbon/react';
import styles from './Card.module.scss';

/// A card: a title at the left and its count at the right on one line, the body under
/// them, and a footer line for what dates the card — the age of the read it came from
/// (`docs/13` §6).
///
/// `href` makes the whole card the way to what it is about, the way a project card is the
/// way to the project; `onClick` lets the router take a route while the browser keeps a
/// modified click and an in-page anchor.
export function Card({
  title,
  count,
  href,
  onClick,
  footer,
  children,
  className,
}: {
  /// The card's name, and anything that belongs on its baseline: a shortname, a key.
  title: ReactNode;
  /// The one number this card is about. Omitted when it has none.
  count?: ReactNode;
  href?: string;
  onClick?: (e: MouseEvent<HTMLAnchorElement> | KeyboardEvent<HTMLAnchorElement>) => void;
  /// The meta line at the foot: when what is above was read.
  footer?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  const cls = [styles.card, className].filter(Boolean).join(' ');
  const body = (
    <>
      <div className={styles.head}>
        <span className={styles.title}>{title}</span>
        {count !== undefined && count !== null && <span className={styles.count}>{count}</span>}
      </div>
      {children !== undefined && children !== null && <div className={styles.body}>{children}</div>}
      {footer !== undefined && footer !== null && <div className={styles.footer}>{footer}</div>}
    </>
  );
  if (href !== undefined) {
    return (
      <ClickableTile href={href} onClick={onClick} className={cls}>
        {body}
      </ClickableTile>
    );
  }
  return <Tile className={cls}>{body}</Tile>;
}

/// A row of cards: three across at `xlg`, two at `lg` and `md`, one in a hand, and every
/// card in a row the height of the tallest — a card is a block, not a ragged column.
export function CardGrid({ label, children, className }: { label?: string; children?: ReactNode; className?: string }) {
  return (
    <div className={[styles.grid, className].filter(Boolean).join(' ')} role={label ? 'group' : undefined} aria-label={label}>
      {children}
    </div>
  );
}
