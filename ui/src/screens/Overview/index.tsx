// New for agent-router: the Overview, at `/` (assets/index.html, the dashboard, rebuilt). Every section the
// dashboard shows, in its order (OV-C1), read as one snapshot per scope every five seconds (OV-C160).
//
// The address is the view (SH-C53): the scope, the range, the request filters and the open request
// rows are its query string. A change of scope or range pushes a history entry, so Back un-scopes
// (OV-C30); a filter or an open row replaces the entry. An old `#a=&p=…` bookmark is not rewritten:
// the clean cut keeps no compatibility code (decision J), and opens the page unscoped.
import { ArrowUp } from '@carbon/icons-react';
import { Button, SkeletonText } from '@carbon/react';
import { useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useTick } from '~/app/tick';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { useAccounts, useAccountsCheck, useAuth, useOverview, type OverviewRead } from '~/data/overview';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { settingsPath } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { readQuery, writeQuery, type OverviewAddress, type RequestFilters, type Scope } from '~/model/scope';
import { networkStatus, opensSettings, type Auth } from '~/model/settings';
import { Activity } from './Activity';
import { buckets } from './timeline';
import { AllAccounts } from './AllAccounts';
import { Headline } from './Headline';
import { Lists } from './Lists';
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
  const { scope, requests: filters, open } = address;
  const read = useOverview(scope);
  const auth = useAuth();
  // The accounts every 30 s, beside the snapshot and never failing it (OV-C166), with no credential
  // checked (OV-C167).
  useAccounts();
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

  // The request list's page, back to twenty on any scope or filter change (OV-C33, OV-C154): the page
  // is kept for the scope and filters it was set under, and any other reads as the first.
  const pageOf = writeQuery({ scope, requests: filters });
  const [paging, setPaging] = useState({ key: '', shown: REQUEST_PAGE });
  const shown = paging.key === pageOf ? paging.shown : REQUEST_PAGE;
  const setShown = (n: number) => setPaging({ key: pageOf, shown: n });

  // A model or stop filter whose value has left the loaded rows is cleared, never left filtering to
  // nothing (OV-C153). Only once this scope's own rows are in: the previous scope's say nothing
  // about this one's.
  const effective = snapshot && !read.pending ? filtering(snapshot.requests, filters, snapshot.summary.latency?.ttfb_p95 ?? null).effective : null;
  useEffect(() => {
    if (!effective) return;
    if (effective.model !== filters.model || effective.stop !== filters.stop) {
      void navigate({ pathname: location.pathname, search: writeQuery({ ...address, requests: { ...filters, model: effective.model, stop: effective.stop } }) }, { replace: true });
    }
  });

  // Bucketed on the clock the summary was asked with (OV-C111).
  const bk = useMemo(() => (snapshot ? buckets(snapshot.summary.timeline, snapshot.tz * 60_000) : []), [snapshot]);
  const busy = read.pending;

  const listErrors = () => {
    setShown(REQUEST_PAGE);
    setFilters({ kind: 'errors', model: null, stop: null });
    requestAnimationFrame(() => document.getElementById(REQUESTS_ID)?.scrollIntoView({ block: 'start', behavior: reducedMotion() ? 'auto' : 'smooth' }));
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
      <ScopeBar scope={scope} snapshot={snapshot} now={now} onScope={setScope} />
      <ProjectPlaces scope={scope} snapshot={snapshot} />
      <SearchPanel scope={scope} onScope={setScope} now={now} />
      {snapshot ? (
        <>
          <Quota snapshot={snapshot} bk={bk} now={now} busy={busy} />
          <AllAccounts forecast={snapshot.forecast} now={now} busy={busy} />
          <Headline snapshot={snapshot} scope={scope} bk={bk} now={now} busy={busy} onListErrors={listErrors} />
          <Activity bk={bk} timeline={snapshot.summary.timeline} now={now} busy={busy} />
          <Reading snapshot={snapshot} scope={scope} busy={busy} />
          <Lists snapshot={snapshot} scope={scope} now={now} busy={busy} auth={auth.value} onScope={setScope} />
          <Requests
            snapshot={snapshot}
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
        </>
      ) : read.freshness === 'connecting' ? (
        <SkeletonText paragraph lineCount={6} />
      ) : (
        // Nothing was ever read: a failure, said as one, never an empty router (SH-C22).
        <p className={styles.problem} role="alert">
          Could not read the router{read.error ? `: ${refusalText(read.error)}` : ''}. Asking again every 5 s.
        </p>
      )}
      <BackToTop />
    </Page>
  );
}
