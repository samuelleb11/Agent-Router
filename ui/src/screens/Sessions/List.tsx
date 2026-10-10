// New for agent-router: the live view's session list (assets/live.html, `loadSessions`, `card`,
// `groupInto`) on Carbon: one ContainedList per project, its rows real buttons, each with a visible
// Archive, and the archived shelf in the open (decision I). A hosted router's sessions from more
// than one machine are headed per machine, as the phones head them (SE-C219), and tabbed: All,
// which heads them so, and a tab for each machine (SE-C226).
import { Fragment, useId, useState } from 'react';
import { Link as RouterLink } from 'react-router';
import { Button, Callout, ContainedList, ContainedListItem, Tab, TabList, TabPanel, TabPanels, Tabs } from '@carbon/react';
import { ChevronDown, ChevronRight } from '@carbon/icons-react';
import { CONCEPTS } from '~/lib/concepts';
import { EmptyState } from '~/components/EmptyState';
import { StatusDot } from '~/components/StatusDot';
import { plural } from '~/model/format';
import { routes, settingsPath } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { CONNECT } from '~/model/machine';
import { emptyList, type GroupView, type ListView, type MachineTab, type MachineView, type Notice, type RowView } from '~/model/session';
import { NoticeLine } from './Notice';
import styles from './Sessions.module.scss';

/// Put away, archived: one icon for one concept, from the registry every screen draws from (SH-C39).
const PUT_AWAY = CONCEPTS.putAway.icon;
/// A machine, as its heading draws it.
const MACHINE = CONCEPTS.machine.icon;

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
/// client when it is Kimi Code's, its age, its title in at most two lines, what it is doing, and its
/// id with its branch (SE-C26, SE-C28, SE-C235).
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
          {/* A Kimi Code session names its client; a Claude Code one names none (SE-C235). */}
          {r.client && (
            <>
              <Said>, </Said>
              <span className={styles.rowClient} data-client="">
                {r.client}
              </span>
            </>
          )}
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

/// One machine's sessions, or the whole list when it is not headed per machine: recent sessions by
/// project, the older fold, and the archived shelf (SE-C29, SE-C32).
function Part({ view, older, onOlder, actions }: { view: ListView; older: boolean; onOlder: () => void; actions: RowActions }) {
  const olderId = useId();
  const shelfId = useId();
  return (
    <>
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
            onClick={onOlder}
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
        <section className={styles.shelf} aria-labelledby={shelfId} data-shelf="">
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
    </>
  );
}

/// A machine's part of a hosted router's list: its heading, and under it what its sessions alone
/// make — its own projects, its own older fold and its own shelf (SE-C219). The heading is drawn
/// whatever the part holds, so a machine with nothing recent still heads its fold or its shelf
/// (SE-C220).
function Machine({ part, level, older, onOlder, actions }: { part: MachineView; level: 2 | 3; older: boolean; onOlder: () => void; actions: RowActions }) {
  const headingId = useId();
  const Heading = level === 2 ? 'h2' : 'h3';
  return (
    <section className={styles.machine} aria-labelledby={headingId} data-machine={part.machine ?? ''}>
      <Heading id={headingId} className={styles.machineHead}>
        <MACHINE aria-hidden="true" className={styles.machineIcon} />
        <span className={styles.machineName}>{part.heading}</span>
      </Heading>
      <Part view={part} older={older} onOlder={onOlder} actions={actions} />
    </section>
  );
}

/// A tab over a list of several machines: its label and, at a glance, what is at work there and
/// what waits on you, with the attention dot when something does (SE-C227). The dot's words are in
/// the counts, which the tab's name carries, so the dot is not heard twice.
function MachineTabLabel({ tab }: { tab: MachineTab }) {
  return (
    <span className={styles.machineTab}>
      {tab.waiting && <StatusDot tone="attention" decorative />}
      <span className={styles.machineTabName}>{tab.label}</span>
      {tab.counts && <span className={styles.machineTabCounts}>{tab.counts}</span>}
    </span>
  );
}

const NO_TABS: readonly MachineTab[] = [];

/// The list: a machine at a time when a hosted router lists more than one, and otherwise the one
/// list with no heading (SE-C219). Over several machines, a row of tabs: All, the machines headed
/// one after another, and a tab for each machine that shows its part alone, its tab naming it in
/// place of its heading (SE-C226). The tab is the address's (SE-C228); arrows move between the
/// tabs and Enter or Space chooses one, so passing over a tab neither draws its machine nor adds a
/// history entry. Each older fold's open state is the list's own, one per machine, kept across
/// every refresh, archive, tab and return to the list (SE-C31): the list stays mounted while a
/// session is open, and only its rows are redrawn. A machine's fold is the same fold on All and on
/// its own tab.
export function List({
  machines,
  hosted,
  wrapperWarning,
  actions,
  level = 2,
  tabs = NO_TABS,
  tab = null,
  onTab,
}: {
  machines: readonly MachineView[];
  hosted: boolean;
  wrapperWarning: string | null;
  actions: RowActions;
  /// The level of a machine's heading: under the screen's own, which is the page's first heading
  /// while the list is what the reader is looking at, and its second beside an open session.
  level?: 2 | 3;
  /// The tabs over a list of several machines (`machineTabs`); none draws the list as All does.
  tabs?: readonly MachineTab[];
  /// The `machine=` of the tab the address is on; null, or one no tab has, is All.
  tab?: string | null;
  onTab?: (machine: string | null) => void;
}) {
  const [opened, setOpened] = useState<ReadonlySet<string>>(() => new Set());
  const warningId = useId();
  const toggle = (key: string) =>
    setOpened((s) => {
      const next = new Set(s);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  const empty = machines.every((m) => !m.recent.length && !m.older && !m.shelf);
  const all = machines.map((m) =>
    m.heading === null ? (
      <Part key={m.key} view={m} older={opened.has(m.key)} onOlder={() => toggle(m.key)} actions={actions} />
    ) : (
      <Machine key={m.key} part={m} level={level} older={opened.has(m.key)} onOlder={() => toggle(m.key)} actions={actions} />
    ),
  );
  const selected = Math.max(0, tabs.findIndex((t) => t.param === tab));
  // A machine's tab draws its part alone, with no heading; All draws every part under its heading.
  const part = machines.find((m) => m.key === tabs[selected]?.key);
  const shown = part ? <Part view={part} older={opened.has(part.key)} onOlder={() => toggle(part.key)} actions={actions} /> : all;
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
        // Which empty it is: a quiet week, or a hosted router with no machine to list, and where to
        // connect one: the Machines screen, the commands' one home (SE-C38, SE-C187, MA-C2).
        <EmptyState
          title={hosted ? 'No sessions here yet' : 'No sessions'}
          action={
            hosted ? (
              <RouterLink className="cds--link" to={routes.machines}>
                {CONNECT}
              </RouterLink>
            ) : null
          }
        >
          {emptyList(hosted)}
        </EmptyState>
      ) : null}
      {tabs.length ? (
        // Chosen by the address alone: Carbon reports a choice through `onChange`, which only
        // writes it there, and an arrow only moves the focus (activation "manual").
        <Tabs
          selectedIndex={selected}
          onChange={({ selectedIndex }) => {
            const next = tabs[selectedIndex];
            if (next && selectedIndex !== selected) onTab?.(next.param);
          }}
        >
          {/* One row that scrolls sideways inside the list, never wider than it (SE-C230). */}
          <TabList aria-label="Machines" activation="manual" className={styles.machineTabs}>
            {tabs.map((t) => (
              <Tab key={t.key || 'all'} aria-label={t.name} data-machine-tab={t.param ?? ''}>
                <MachineTabLabel tab={t} />
              </Tab>
            ))}
          </TabList>
          {/* Every tab has its panel, labelled by the tab; only the chosen one draws its rows. */}
          <TabPanels>
            {tabs.map((t, i) => (
              <TabPanel key={t.key || 'all'} className={styles.machinePanel}>
                {i === selected ? shown : null}
              </TabPanel>
            ))}
          </TabPanels>
        </Tabs>
      ) : (
        all
      )}
    </div>
  );
}
