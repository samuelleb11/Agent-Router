// New for agent-router: the live view's session tabs (assets/live.html, `drawTabs`) as a strip of
// links, not Carbon's Tabs: those are capped at one 64px row, wrap out of reach on a phone (Northstar
// 29472eb), and their dismissable form hard-codes a Close glyph where put-away is Archive (decision I).
import { useEffect, useRef } from 'react';
import { Link, useLocation } from 'react-router';
import { Button, Toggletip, ToggletipButton, ToggletipContent } from '@carbon/react';
import { Information } from '@carbon/icons-react';
import { CONCEPTS } from '~/lib/concepts';
import { StatusDot } from '~/components/StatusDot';
import { useCoarse } from '~/lib/viewport';
import { machineIn, withMachine, type Notice, type TabView } from '~/model/session';
import { NoticeLine } from './Notice';
import styles from './Sessions.module.scss';

/// Put away, archived: one icon for one concept, from the registry every screen draws from (SH-C39).
const PUT_AWAY = CONCEPTS.putAway.icon;

/// The strip over the feed: the project's current sessions, oldest first, each with its dot and its
/// words, the open one marked (SE-C49); one row that scrolls sideways and brings the open tab into
/// view (SE-C54); and on every tab a visible Archive, a target of its own (SE-C50). A tab keeps the
/// list's machine tab in its address, so the list beside it stays where it is (SE-C228).
export function Strip({
  tabs,
  onArchive,
  busy,
  notice,
  onNoticeClose,
}: {
  tabs: readonly TabView[];
  onArchive: (id: string) => void;
  busy: ReadonlySet<string>;
  notice: Notice | null;
  onNoticeClose: () => void;
}) {
  const coarse = useCoarse();
  const machine = machineIn(useLocation().search);
  const open = useRef<HTMLAnchorElement>(null);
  const openId = tabs.find((t) => t.open)?.id;
  useEffect(() => {
    open.current?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [openId]);
  return (
    <nav className={styles.strip} aria-label="This project’s sessions">
      <div className={styles.stripRow}>
        <ul className={styles.tabs}>
          {tabs.map((t) => (
            <li key={t.id} className={styles.tab} data-open={t.open || undefined}>
              <Link
                ref={t.open ? open : undefined}
                to={withMachine(`?session=${encodeURIComponent(t.id)}`, machine)}
                className={styles.tabLink}
                aria-current={t.open ? 'page' : undefined}
              >
                <StatusDot tone={t.tone} label={t.spoken} decorative />
                <span className={styles.tabText}>
                  <span className={styles.tabLabel}>{t.label}</span>
                  {/* What it is doing, as words: never a hover title (SE-C20, SE-C204). */}
                  <span className={styles.tabWords}>{t.spoken}</span>
                </span>
              </Link>
              <Button
                kind="ghost"
                size={coarse ? 'lg' : 'sm'}
                className={styles.tabArchive}
                hasIconOnly
                renderIcon={PUT_AWAY}
                iconDescription={`Archive ${t.label}`}
                tooltipPosition="bottom"
                disabled={busy.has(t.id)}
                onClick={() => onArchive(t.id)}
              />
            </li>
          ))}
        </ul>
        <Toggletip align="bottom-end" className={styles.stripHelp}>
          <ToggletipButton label="What Archive does">
            <Information />
          </ToggletipButton>
          <ToggletipContent>
            <p>Archive puts a session away: it leaves these tabs and the list until it shows life again. The open session keeps its tab while it is open.</p>
          </ToggletipContent>
        </Toggletip>
      </div>
      <NoticeLine notice={notice} onClose={onNoticeClose} />
    </nav>
  );
}
