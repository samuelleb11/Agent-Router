// Copied from Northstar ui/src/components/PageHeader.tsx at e9c1589.
// Changed: nothing. Its docs/13 is Northstar's console contracts, which bind here too (docs/web-console-carbon.md).
import type { ReactNode } from 'react';
import { Link as RouterLink } from 'react-router';
import { Breadcrumb, BreadcrumbItem } from '@carbon/react';
import styles from './PageHeader.module.scss';

/// One parent in the trail. The last entry is the page itself and is never a link.
export type Crumb = { label: ReactNode; to?: string };

/// Every screen opens the same way, and this is the only thing that builds that opening.
///
/// **Why not Carbon's `PageHeader`.** @carbon/react 1.116 ships it, but only as
/// `unstable__PageHeader` / `preview__PageHeader`: an unstable API in a console whose
/// screens carry behavioural contracts. Its `Content`/`ContentText` slots also impose their
/// own two-column arrangement, which puts the page action in a fixed place and leaves no
/// baseline for the shortname tag beside the title. Breadcrumb plus one CSS grid gives the
/// exact frame the console needs and adds no Carbon partial to `styles.scss`.
///
/// **Why the action is last in the DOM.** On a desk it belongs at the right of the title
/// row; on a phone it is full width directly under the meta line. That is pure layout, so
/// it adapts in CSS (`docs/13-console-contracts.md` §7) — named grid areas put one DOM
/// order in both places, and nothing here reads a breakpoint. Source order is the phone's,
/// which is also the reading order a screen reader gets: title, then what is true, then
/// what you can do about it.
export function PageHeader({
  breadcrumb,
  crumbAction,
  title,
  titleHint,
  tags,
  meta,
  links,
  action,
  dense = false,
  className,
}: {
  /// Parents as links, this page last. Omitted on a screen with no trail.
  breadcrumb?: readonly Crumb[];
  /// One affordance beside the trail, for a screen where the trail is a fact you can
  /// change rather than only a way back — the item page, where the crumbs ARE the item's
  /// ancestry and moving it rewrites them.
  crumbAction?: ReactNode;
  /// The page title. A node, so a screen can show a skeleton while the record is unread.
  title: ReactNode;
  /// A hover explanation for the title — used when the title is standing in for a record
  /// that could not be read.
  titleHint?: string;
  /// Tags and identifiers beside the title, on its baseline: a shortname, a key.
  tags?: ReactNode;
  /// THE meta line: counts and the read age, in one line under the title. One line, not
  /// two — everything a screen wants said there goes in this node.
  meta?: ReactNode;
  /// Secondary destinations, as Links and never buttons; they sit in the meta line.
  links?: ReactNode;
  /// The primary action, a Button element.
  action?: ReactNode;
  /// For a screen whose content needs the window, the Work map's drawing: a smaller title, and
  /// on a desk the meta line on the title's row, wrapping under it only when the row is full.
  /// The same DOM order, so in a hand it stacks as every header does.
  dense?: boolean;
  className?: string;
}) {
  const trail = breadcrumb ?? [];
  const metaLine = (meta !== undefined && meta !== null) || (links !== undefined && links !== null);
  return (
    <header className={[styles.header, dense && styles.dense, className].filter(Boolean).join(' ')}>
      {trail.length > 0 && (
        <div className={styles.crumbs}>
          <Breadcrumb noTrailingSlash>
            {trail.map((c, i) => {
              const current = i === trail.length - 1;
              return (
                <BreadcrumbItem key={i} isCurrentPage={current}>
                  {c.to !== undefined && !current ? <RouterLink to={c.to}>{c.label}</RouterLink> : c.label}
                </BreadcrumbItem>
              );
            })}
          </Breadcrumb>
          {crumbAction !== undefined && crumbAction !== null && <span className={styles.crumbAction}>{crumbAction}</span>}
        </div>
      )}

      <div className={styles.titleRow}>
        <h1 className={styles.title} title={titleHint}>
          {title}
        </h1>
        {tags !== undefined && tags !== null && <span className={styles.tags}>{tags}</span>}
      </div>

      {metaLine && (
        <div className={styles.meta}>
          {meta}
          {links !== undefined && links !== null && (
            <nav className={styles.links} aria-label="This page">
              {links}
            </nav>
          )}
        </div>
      )}

      {action !== undefined && action !== null && <div className={styles.action}>{action}</div>}
    </header>
  );
}
