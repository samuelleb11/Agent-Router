// New for agent-router: the live view's session list (assets/live.html, `loadSessions`, `card`,
// `groupInto`) on Carbon: one ContainedList per project, its rows real buttons, each with a visible
// Archive, and the archived shelf in the open (decision I).
import { Fragment, useId, useState } from 'react';
import { Link as RouterLink } from 'react-router';
import { Button, Callout, ContainedList, ContainedListItem } from '@carbon/react';
import { ChevronDown, ChevronRight } from '@carbon/icons-react';
import { CONCEPTS } from '~/lib/concepts';
import { EmptyState } from '~/components/EmptyState';
import { StatusDot } from '~/components/StatusDot';
import { plural } from '~/model/format';
import { settingsPath } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { emptyList, type GroupView, type ListView, type Notice, type RowView } from '~/model/session';
import { NoticeLine } from './Notice';
import styles from './Sessions.module.scss';

/// Put away, archived: one icon for one concept, from the registry every screen draws from (SH-C39).
const PUT_AWAY = CONCEPTS.putAway.icon;

export interface RowActions {
  /// Open a session: the row itself does this, and its action never does (SE-C34).
  readonly open: (id: string) => void;
  /// Put a session away, or bring it back.
  readonly archive: (id: string, archived: boolean) => void;
  readonly busy: ReadonlySet<string>;
  /// Each row's own notice: an archive's outcome sits by its row (SE-C178, SE-C179).
  readonly notices: ReadonlyMap<string, Notice>;
  readonly dismiss: (id: string) => void;
  /// The session open beside the list, marked in it (SE-C201).
  readonly openId: string | null;
}

/// Words a screen reader hears between a row's parts, which the eye tells apart by their places.
const Said = ({ children }: { children: string }) => <span className={styles.said}>{children}</span>;

/// One session's row: its dot with the halo when live, its name, whether it can be typed into, its
/// age, its title in at most two lines, what it is doing, and its id with its branch (SE-C26, SE-C28).
function Row({ r, actions, shelved }: { r: RowView; actions: RowActions; shelved: boolean }) {
  const coarse = useCoarse();
  const notice = actions.notices.get(r.id) ?? null;
  const open = actions.openId === r.id;
  return (
    <Fragment>
      <ContainedListItem
        className={[styles.row, shelved ? styles.shelved : '', open ? styles.openRow : ''].filter(Boolean).join(' ')}
        aria-current={open ? 'true' : undefined}
        data-session={r.id}
        onClick={() => actions.open(r.id)}
        action={
          <Button
            kind="ghost"
            size={coarse ? 'lg' : 'sm'}
            renderIcon={PUT_AWAY}
            className={styles.rowAction}
            disabled={actions.busy.has(r.id)}
            aria-label={`${shelved ? 'Restore' : 'Archive'} ${r.title ?? r.name}`}
            onClick={() => actions.archive(r.id, !shelved)}
          >
            {shelved ? 'Restore' : 'Archive'}
          </Button>
        }
      >
        <span className={styles.rowTop}>
          <StatusDot tone={r.tone} live={r.halo} label={r.label} />
          <Said>, </Said>
          <span className={styles.rowName}>{r.name}</span>
          <Said>, </Said>
          <span className={styles.reach} data-reach={r.reach}>
            {r.reach}
          </span>
          <Said>, moved </Said>
          <span className={styles.rowAge}>{r.age}</span>
          <Said> ago</Said>
        </span>
        {r.title && (
          <span className={styles.rowTitle}>
            <Said>, </Said>
            {r.title}
          </span>
        )}
        {/* Coloured only when it needs a reader: waiting, stalled, failed (SE-C27). */}
        {r.line && (
          <span className={styles.rowLine} data-tone={r.lineTone ?? undefined}>
            <Said>, </Said>
            {r.line}
          </span>
        )}
        <span className={styles.rowMeta}>
          <Said>, </Said>
          {r.meta}
        </span>
      </ContainedListItem>
      {notice && (
        <li className={styles.rowNotice}>
          <NoticeLine notice={notice} onClose={() => actions.dismiss(r.id)} />
        </li>
      )}
    </Fragment>
  );
}

/// A project's heading: its most pressing session's dot, its name, how many sessions it holds and
/// how long since its newest moved (SE-C18, SE-C25).
function Group({ g, actions }: { g: GroupView; actions: RowActions }) {
  const coarse = useCoarse();
  return (
    <ContainedList
      className={styles.group}
      kind="on-page"
      size={coarse ? 'xl' : 'lg'}
      label={
        <span className={styles.groupHead}>
          <StatusDot tone={g.tone} live={g.halo} label={g.label} />
          <span className={styles.groupName}>{g.key}</span>
          <span className={styles.groupCount}>{plural(g.count, 'session', 'sessions')}</span>
          <span className={styles.groupAge}>{g.age}</span>
        </span>
      }
    >
      {g.rows.map((r) => (
        <Row key={r.id} r={r} actions={actions} shelved={false} />
      ))}
    </ContainedList>
  );
}

/// The list: recent sessions by project, the older fold, and the archived shelf. The fold's open
/// state is the list's own, kept across every refresh, archive and return to the list (SE-C31): the
/// list stays mounted while a session is open, and only its rows are redrawn.
export function List({
  view,
  hosted,
  wrapperWarning,
  actions,
}: {
  view: ListView;
  hosted: boolean;
  wrapperWarning: string | null;
  actions: RowActions;
}) {
  const [older, setOlder] = useState(false);
  const olderId = useId();
  const shelfId = useId();
  const warningId = useId();
  const empty = !view.recent.length && !view.older && !view.shelf;
  return (
    <div className={styles.list}>
      {wrapperWarning && (
        // An editor update ships an unwrapped binary, and nothing else would say so (SE-C39). The
        // sentence names the Settings that install the wrapper, and links there (SH-C6): a Callout,
        // since Carbon's inline notification takes no link.
        <Callout kind="warning" lowContrast className={styles.warning} title={wrapperWarning} titleId={warningId}>
          <RouterLink className="cds--link" to={settingsPath('claude-code')} aria-describedby={warningId}>
            Open Settings → Claude Code
          </RouterLink>
        </Callout>
      )}
      {empty ? (
        // Which empty it is: a quiet week, or a hosted router with no machine to list (SE-C38).
        <EmptyState title={hosted ? 'No sessions here yet' : 'No sessions'}>{emptyList(hosted)}</EmptyState>
      ) : null}
      {view.recent.map((g) => (
        <Group key={g.key} g={g} actions={actions} />
      ))}
      {view.older && (
        <div className={styles.older}>
          <Button
            kind="ghost"
            size="md"
            className={styles.fold}
            renderIcon={older ? ChevronDown : ChevronRight}
            aria-expanded={older}
            aria-controls={olderId}
            onClick={() => setOlder(!older)}
          >
            {view.older.label}
          </Button>
          {older && (
            <div id={olderId} className={styles.olderGroups}>
              {view.older.groups.map((g) => (
                <Group key={g.key} g={g} actions={actions} />
              ))}
            </div>
          )}
        </div>
      )}
      {view.shelf && (
        // Visible, not folded: how many it holds, flat and newest first, each dimmed (SE-C32).
        <section className={styles.shelf} aria-labelledby={shelfId}>
          <ContainedList
            kind="on-page"
            size="lg"
            label={
              <span id={shelfId} className={styles.groupHead}>
                <PUT_AWAY aria-hidden="true" />
                <span className={styles.groupName}>{view.shelf.label}</span>
              </span>
            }
          >
            {view.shelf.rows.map((r) => (
              <Row key={r.id} r={r} actions={actions} shelved />
            ))}
          </ContainedList>
        </section>
      )}
    </div>
  );
}
