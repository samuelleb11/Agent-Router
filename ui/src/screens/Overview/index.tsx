// New for agent-router: the Overview, at `/` (assets/index.html, the dashboard, rebuilt). Quota headroom,
// All accounts and Totals in scope on top, and every other section the dashboard shows in a tab under
// them (OV-C1), read every five seconds as one snapshot of the top and the open tab, and nothing for a
// tab that is not open (OV-C160, OV-C216).
//
// The address is the view (SH-C53): the scope, the range, the open tab, the request filters and the
// open request rows are its query string. A change of scope, range or tab pushes a history entry, so
// Back un-scopes (OV-C30) and goes back to the last tab (OV-C215); a filter or an open row replaces the
// entry. An old `#a=&p=…` bookmark is not rewritten: the clean cut keeps no compatibility code
// (decision J), and opens the page unscoped.
import { ArrowUp } from '@carbon/icons-react';
import { Button, SkeletonText, Tab, TabList, TabPanel, TabPanels, Tabs } from '@carbon/react';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useTick } from '~/app/tick';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { useAccounts, useAccountsCheck, useAuth, useOverview, type OverviewRead } from '~/data/overview';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { settingsPath } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { readQuery, TABS, writeQuery, type OverviewAddress, type OverviewTab, type RequestFilters, type Scope } from '~/model/scope';
import { networkStatus, opensSettings, type Auth } from '~/model/settings';
import { Activity } from './Activity';
import { buckets } from './timeline';
import { AllAccounts } from './AllAccounts';
import { Headline } from './Headline';
import { Lists, Tools } from './Lists';
import { Quota } from './Quota';
import { Reading } from './Reading';
import { Requests, REQUESTS_ID } from './Requests';
import { filtering, REQUEST_PAGE } from './requestList';
import { ProjectPlaces } from './ProjectPlaces';
import { ScopeBar } from './ScopeBar';
import { SearchPanel } from './Search';
import styles from './Overview.module.scss';

/// Whether this load has decided if Settings → Accounts opens by itself. It decides at most once,
/// either way (ST-C7, OV-C209), so a reader who leaves Settings is never dragged back. Per load, so
/// module state: a remount is not a new load.
let settingsDecided = false;

/// The location the Overview first drew this load: the one place it may interrupt the reader, before
/// they have gone anywhere from it (ST-C7).
let firstDrawn: string | null = null;

const reducedMotion = (): boolean => globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;

/// What each tab is called, in the tab row's order (OV-C1).
const TAB_NAMES: Readonly<Record<OverviewTab, string>> = {
  activity: 'Activity',
  context: 'Context',
  projects: 'Projects & sessions',
  tools: 'Tools',
  requests: 'Requests',
  search: 'Search',
};

/// "read 3s ago", kept moving between polls (OV-C173), or the stale and unreachable sentences, each
/// with the time of the last good read (OV-C164).
function Freshness({ read }: { read: OverviewRead }) {
  useTick(1000);
  const last = read.lastOk === null ? 'never read successfully' : `showing what was read ${since(read.lastOk)}`;
  const reason = read.error ? `: ${refusalText(read.error)}` : '';
  const words =
    read.freshness === 'connecting'
      ? 'reading the router…'
      : read.freshness === 'live'
        ? `read ${since(read.lastOk)}, every 5 s`
        : read.freshness === 'stale'
          ? `stale — the last ${read.strikes === 1 ? 'refresh' : `${read.strikes} refreshes`} failed${reason}; ${last}`
          : `unreachable — ${read.strikes} refreshes in a row failed${reason}; ${last}`;
  return (
    <span className={styles.freshness} role="status" data-freshness={read.freshness}>
      {words}
      {read.pending ? ' · reading the new scope…' : ''}
    </span>
  );
}

/// While the router listens beyond loopback, the page says so, and whether a password guards it,
/// in the words of the router it is (SH-C18, SH-C19, OV-C208).
function Network({ auth }: { auth: Auth | null }) {
  const status = networkStatus(auth);
  if (!status) return null;
  return (
    <span className={styles.network} data-network={status.tone}>
      <span className={styles.dot} data-tone={status.tone} aria-hidden="true" />
      <span>{status.text}</span>
      <span>— {status.explanation}</span>
    </span>
  );
}

/// "Back to top", once the page is 600 px down, smoothly unless reduced motion is asked for
/// (SH-C48, OV-C200).
function BackToTop() {
  const coarse = useCoarse();
  const [down, setDown] = useState(() => (globalThis.scrollY ?? 0) >= 600);
  useEffect(() => {
    const on = () => setDown(window.scrollY >= 600);
    window.addEventListener('scroll', on, { passive: true });
    return () => window.removeEventListener('scroll', on);
  }, []);
  if (!down) return null;
  return (
    <Button
      className={styles.toTop}
      kind="secondary"
      size={coarse ? 'lg' : 'md'}
      renderIcon={ArrowUp}
      onClick={() => window.scrollTo({ top: 0, behavior: reducedMotion() ? 'auto' : 'smooth' })}
    >
      Back to top
    </Button>
  );
}

export default function Overview() {
  const location = useLocation();
  const navigate = useNavigate();
  const address = useMemo(() => readQuery(location.search), [location.search]);
  const { scope, tab, requests: filters, open } = address;
  const read = useOverview(scope, tab);
  const auth = useAuth();
  // The accounts every 30 s, beside the snapshot and never failing it (OV-C166), with no credential
  // checked (OV-C167). They name the account that served a request (OV-C231), and say whose a scoped
  // account is when a provider is chosen (OV-C227).
  const accounts = useAccounts();
  // The page as this load first drew it, before the reader has gone anywhere: only from there does
  // the console interrupt (ST-C7). Kept per load, so a remount does not move it.
  const [first] = useState(() => (firstDrawn ??= location.key));
  const opening = location.key === first;
  const checked = useAccountsCheck(opening);
  const snapshot = read.snapshot;
  const now = Date.now();

  // SH-C7: the Overview is "Agent Router" in a tab, the history list and an editor's tab.
  useEffect(() => {
    document.title = 'Agent Router';
  }, []);

  // Settings → Accounts opens by itself at most once per load, only from the Overview as the load
  // opened it, when nothing can sign or a credential that could sign does not work; on a hosted
  // router only the accounts it could sign with count. The mode is read first, because it decides
  // what may interrupt (ST-C7, ST-C8, OV-C166, OV-C209).
  useEffect(() => {
    if (settingsDecided || !opening || !auth.value || !checked) return;
    settingsDecided = true;
    if (opensSettings(checked, auth.value)) void navigate(settingsPath('accounts'));
  }, [opening, auth.value, checked, navigate]);

  const go = (next: Partial<OverviewAddress>, replace: boolean) => void navigate({ pathname: location.pathname, search: writeQuery({ ...address, ...next }) }, { replace });
  /// A scope or a range: a history entry of its own (OV-C30), read at once (OV-C33).
  const setScope = (next: Scope) => go({ scope: next }, false);
  const setFilters = (next: RequestFilters) => go({ requests: next }, true);
  const setOpen = (ids: readonly string[]) => go({ open: ids }, true);
  /// A tab: a history entry of its own, read at once (OV-C215).
  const setTab = (next: OverviewTab) => {
    if (next !== tab) go({ tab: next }, false);
  };

  // The request list's page, back to twenty on any scope or filter change (OV-C33, OV-C154): the page
  // is kept for the scope and filters it was set under, and any other reads as the first.
  const pageOf = writeQuery({ scope, requests: filters });
  const [paging, setPaging] = useState({ key: '', shown: REQUEST_PAGE });
  const shown = paging.key === pageOf ? paging.shown : REQUEST_PAGE;
  const setShown = (n: number) => setPaging({ key: pageOf, shown: n });

  // The open tab's own figures, once they are in: until then the snapshot holds another tab's.
  const part = snapshot?.part.tab === tab ? snapshot.part : null;

  // A model or stop filter whose value has left the loaded rows is cleared, never left filtering to
  // nothing (OV-C153). Only once this scope's own rows are in: the previous scope's say nothing
  // about this one's.
  const effective = snapshot && part?.tab === 'requests' && !read.pending ? filtering(part.requests, filters, snapshot.summary.latency?.ttfb_p95 ?? null).effective : null;
  useEffect(() => {
    if (!effective) return;
    if (effective.model !== filters.model || effective.stop !== filters.stop) {
      void navigate({ pathname: location.pathname, search: writeQuery({ ...address, requests: { ...filters, model: effective.model, stop: effective.stop } }) }, { replace: true });
    }
  });

  // Bucketed on the clock the summary was asked with (OV-C111).
  const bk = useMemo(() => (snapshot ? buckets(snapshot.summary.timeline, snapshot.tz * 60_000) : []), [snapshot]);
  const busy = read.pending;

  // "List the N errors" opens the Requests tab filtered to errors, and brings the list into view once
  // it is drawn, which on another tab is after its rows are read (OV-C107).
  const toRequests = useRef(false);
  const listErrors = () => {
    setShown(REQUEST_PAGE);
    go({ tab: 'requests', requests: { kind: 'errors', model: null, stop: null } }, tab === 'requests');
    toRequests.current = true;
  };
  useEffect(() => {
    const list = toRequests.current ? document.getElementById(REQUESTS_ID) : null;
    if (!list) return;
    toRequests.current = false;
    requestAnimationFrame(() => list.scrollIntoView({ block: 'start', behavior: reducedMotion() ? 'auto' : 'smooth' }));
  });

  // The open tab's body: Activity draws from the top's timeline whichever tab the snapshot was read
  // for; the others wait for their own figures, said as being read rather than drawn empty.
  const body = (): ReactNode => {
    if (!snapshot) return null;
    if (tab === 'activity') return <Activity bk={bk} timeline={snapshot.summary.timeline} now={now} busy={busy} />;
    if (!part) return <SkeletonText paragraph lineCount={4} />;
    switch (part.tab) {
      case 'context':
        return <Reading summary={snapshot.summary} part={part} scope={scope} busy={busy} />;
      case 'projects':
        return <Lists part={part} unattributed={snapshot.summary.unattributed} scope={scope} now={now} busy={busy} auth={auth.value} onScope={setScope} />;
      case 'tools':
        return <Tools tools={part.tools} scope={scope} busy={busy} />;
      case 'requests':
        return (
          <Requests
            requests={part.requests}
            accounts={accounts.value}
            p95={snapshot.summary.latency?.ttfb_p95 ?? null}
            scope={scope}
            filters={filters}
            open={open}
            now={now}
            busy={busy}
            shown={shown}
            onShown={setShown}
            onFilters={setFilters}
            onOpen={setOpen}
          />
        );
      default:
        return null;
    }
  };

  return (
    <Page>
      <PageHeader
        title="Overview"
        meta={
          <span className={styles.meta}>
            <Freshness read={read} />
            <Network auth={auth.value} />
          </span>
        }
      />
      <ScopeBar scope={scope} snapshot={snapshot} accounts={accounts.value} now={now} onScope={setScope} />
      <ProjectPlaces scope={scope} snapshot={snapshot} />
      {snapshot ? (
        <>
          <Quota snapshot={snapshot} bk={bk} now={now} busy={busy} />
          <AllAccounts forecast={snapshot.forecast} now={now} busy={busy} />
          <Headline snapshot={snapshot} scope={scope} bk={bk} now={now} busy={busy} onListErrors={listErrors} />
        </>
      ) : read.freshness === 'connecting' ? (
        <SkeletonText paragraph lineCount={6} />
      ) : (
        // Nothing was ever read: a failure, said as one, never an empty router (SH-C22).
        <p className={styles.problem} role="alert">
          Could not read the router{read.error ? `: ${refusalText(read.error)}` : ''}. Asking again every 5 s.
        </p>
      )}
      {/* Chosen by the address alone: Carbon reports a choice through `onChange`, which only writes it
          there, and an arrow only moves the focus (activation "manual"), as Sessions' machine tabs do. */}
      <Tabs
        selectedIndex={TABS.indexOf(tab)}
        onChange={({ selectedIndex }) => {
          const next = TABS[selectedIndex];
          if (next) setTab(next);
        }}
      >
        <TabList aria-label="Overview sections" activation="manual" className={styles.tabs}>
          {TABS.map((t) => (
            <Tab key={t} data-overview-tab={t}>
              {TAB_NAMES[t]}
            </Tab>
          ))}
        </TabList>
        {/* Only the open tab draws, and only it was read (OV-C216). Search stays mounted, hidden, so what
            was asked survives a tab change and "/" reaches it from any tab (OV-C37). */}
        <TabPanels>
          {TABS.map((t) => (
            <TabPanel key={t} className={styles.tabPanel}>
              {t === 'search' ? <SearchPanel scope={scope} onScope={setScope} now={now} shown={tab === 'search'} onShow={() => setTab('search')} /> : t === tab ? body() : null}
            </TabPanel>
          ))}
        </TabPanels>
      </Tabs>
      <BackToTop />
    </Page>
  );
}
