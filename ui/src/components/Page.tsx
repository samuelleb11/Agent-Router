// Copied from Northstar ui/src/components/Page.tsx at e9c1589.
// Changed: nothing. Its docs/13 is Northstar's console contracts, which bind here too (docs/web-console-carbon.md).
import { useId, type ReactNode } from 'react';
import styles from './Page.module.scss';

/// The page frame: one left edge for everything on the screen.
///
/// **Why a CSS grid on spacing tokens rather than Carbon's `<Grid>`/`<Column>`.** Carbon's
/// CSS grid is a 16-column subgrid whose margins are the breakpoint's own (16px below
/// `max`, 24px at `max`) and whose columns carry half-gutter inline padding. Two things
/// follow that this console cannot have: the page gutter would be Carbon's margin rather
/// than the $spacing-07/$spacing-05 the console's rhythm is built on, and every block would
/// start at its column's padding edge instead of one shared left edge — a table's toolbar
/// and a tile's label would sit 8px apart. The columns buy nothing here either: every
/// section on every screen is full width. So the frame is one CSS grid on Carbon's spacing
/// tokens, and `@carbon/react/scss/grid` stays out of `styles.scss`. Nothing inside a page
/// is stopped from using Carbon's grid if a screen ever needs real columns.
///
/// The page is fluid to Carbon's `max` grid width and stays left-aligned: this console has
/// a rail at the left, and centring the content at ultrawide would take the page away from
/// it. `docs/13-console-contracts.md` §7: one product on two platforms — the gutter is the
/// only thing that changes on a phone.
export function Page({ children, className }: { children?: ReactNode; className?: string }) {
  return <div className={join(styles.page, className)}>{children}</div>;
}

/// One section of a page: an optional heading at the left edge, an optional control at the
/// right of that heading row, and the section's content under it.
///
/// A section with a `title` is labelled by its own heading; one without takes `label`, so a
/// region of a page is never an unnamed landmark. `describedBy` points at the line that
/// dates what the section shows — a count is only as true as the read it came from
/// (`docs/13` §6).
function Section({
  id,
  title,
  label,
  describedBy,
  action,
  children,
  className,
}: {
  /// A scroll target, for the in-page links a tile or a queue points at.
  id?: string;
  /// The section heading, rendered as the one `heading-03` on the page's second level.
  title?: ReactNode;
  /// The accessible name for a section that shows no heading.
  label?: string;
  /// The id of the element dating this section's numbers.
  describedBy?: string;
  /// A control at the right of the heading row; wraps under it when there is no room.
  action?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  const headingId = useId();
  const titled = title !== undefined && title !== null;
  return (
    <section
      id={id}
      className={join(styles.section, className)}
      aria-labelledby={titled ? headingId : undefined}
      aria-label={titled ? undefined : label}
      aria-describedby={describedBy}
    >
      {(titled || action !== undefined) && (
        <div className={styles.sectionHead}>
          {titled && (
            <h2 id={headingId} className={styles.h2}>
              {title}
            </h2>
          )}
          {action !== undefined && action !== null && <div className={styles.sectionAction}>{action}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

Page.Section = Section;

function join(...parts: (string | undefined)[]): string {
  return parts.filter(Boolean).join(' ');
}
