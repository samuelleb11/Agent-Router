// New for agent-router: the Sessions screen at `/live`, the live view (assets/live.html) rebuilt on
// Carbon inside the shell. The list of sessions by project, and beside it on a desk (decision H) the
// open session: its stream, tabs, facts, feed, prompt and composer. The address is the view
// (SE-C189, SE-C175): `?machine=<id>` is the list's tab on a hosted router listing several machines
// (SE-C228), `session=<id>` the open session, `&block=&ordinal=` the entry a search hit arrived at,
// and `find=` with `scope=` is Find.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useSearchParams } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, Toggletip, ToggletipButton, ToggletipContent } from '@carbon/react';
import { Information, Search } from '@carbon/icons-react';
import { EmptyState } from '~/components/EmptyState';
import { StatusPill } from '~/components/StatusDot';
import { useRead } from '~/data/held';
import { liveKeys, postLive, readHosted, readSessions, readShim, type Outcome } from '~/data/live';
import { refusalText } from '~/lib/errors';
import { POLL_MS, since } from '~/lib/freshness';
import type { FindHit, FindScope, SearchHit } from '~/model/search';
import {
  MACHINE_UNLISTED,
  OFF_MACHINE,
  byMachine,
  header,
  hitTarget,
  listsMachine,
  machineTabs,
  notice,
  withMachine,
  wrapperWarning,
  type Notice,
  type SessionRow,
} from '~/model/session';
import { Find } from './Find';
import { KeepContext } from './keep';
import { List, type RowActions } from './List';
import { arrivalIn, failed } from './logic';
import { NoticeLine } from './Notice';
import { useNotice } from './useNotice';
import { SessionView } from './SessionView';
import { StartSession } from './StartSession';
import { useViewportPin } from './useViewportPin';
import styles from './Sessions.module.scss';

const TITLE = 'Live sessions · Agent Router';

/// What the reader is looking at, as a notice raised there remembers it: a notice belongs to the
/// view it was raised in, and opening a session or going back to the list clears it (SE-C182).
const viewOf = (session: string | null, find: boolean) => (find ? 'find' : (session ?? 'list'));

/// `/live`. The screen keeps the list mounted beside or under whatever else is open, so the older
/// fold keeps its state (SE-C31); the open session is keyed on its id (SE-C192).
export default function Sessions() {
  useEffect(() => {
    document.title = TITLE;
  }, []);
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const location = useLocation();
  const client = useQueryClient();
  const [keep] = useState(() => new Map<string, unknown>());
  const root = useRef<HTMLDivElement>(null);
  useViewportPin(root);

  const session = params.get('session') || null;
  // The list's tab, kept by every address the screen writes, so opening a session, Find and going
  // back return to it (SE-C228).
  const machine = params.get('machine') || null;
  const findOpen = params.has('find');
  const query = params.get('find') ?? '';
  const scope: FindScope = session && params.get('scope') !== 'all' ? 'session' : 'all';
  const arrival = useMemo(() => (session ? arrivalIn(params) : null), [params, session]);
  const view = viewOf(session, findOpen);

  // ---------- reads ----------
  // The list, every five seconds on the console's one clock and whenever the page is looked at
  // again; never retried quietly (SE-C40, SH-C15).
  const list = useRead({ queryKey: liveKeys.sessions, queryFn: ({ signal }) => readSessions(signal), refetchInterval: POLL_MS, retry: false, staleTime: 0 });
  // What the router is, from /api/auth's `mode` (SE-C186); the wrapper's state, never asked of a
  // hosted router (SE-C188).
  const hostedRead = useQuery({ queryKey: liveKeys.hosted, queryFn: ({ signal }) => readHosted(signal), staleTime: Infinity, retry: false });
  const hosted = hostedRead.data === true;
  const shim = useQuery({ queryKey: liveKeys.shim, queryFn: ({ signal }) => readShim(signal), enabled: hostedRead.data === false, refetchInterval: 30_000, retry: false, staleTime: 0 });

  const rows = useMemo(() => list.value ?? [], [list.value]);
  // A machine at a time when a hosted router lists more than one, else the one list (SE-C219), and
  // then a tab for All and one for each machine (SE-C226).
  const shown = useMemo(() => byMachine(rows, Date.now()), [rows]);
  const tabs = useMemo(() => machineTabs(rows, shown), [rows, shown]);
  // The last row each session was listed with: an open session keeps its tab and its facts when a
  // later read no longer lists it (SE-C45).
  const known = useRef(new Map<string, SessionRow>());
  useEffect(() => {
    for (const r of rows) known.current.set(r.session_id, r);
  }, [rows]);
  const row = session ? (rows.find((r) => r.session_id === session) ?? known.current.get(session) ?? null) : null;

  // ---------- notices that belong to a view ----------
  const [listSaid, sayList] = useNotice();
  const [listSaidFor, setListSaidFor] = useState<string | null>(null);
  const raise = useCallback(
    (n: Notice | null, at: string | null) => {
      setListSaidFor(n ? at : null);
      sayList(n);
    },
    [sayList],
  );
  // Leaving the view a notice was raised in clears it; arriving there shows it.
  const lastView = useRef(view);
  useEffect(() => {
    const was = lastView.current;
    lastView.current = view;
    if (was !== view && listSaidFor === was) raise(null, null);
  }, [view, listSaidFor, raise]);

  // A URL naming a session the router does not list lands on the list, and says so (SE-C191); one
  // naming a machine the list does not hold now — it went offline, or the link is old — lands on
  // All, and says so, rather than on an empty tab (SE-C229). Each list read must be one made since
  // what it names was asked for. A session is looked for once, as an open one keeps its tab when a
  // later read drops it (SE-C45); a machine at every read, so one that leaves while its tab is open
  // leaves it the same way. An old link can name both, so both are settled in one address.
  const askedAt = useRef<{ id: string; at: number } | null>(null);
  if (session && askedAt.current?.id !== session) askedAt.current = { id: session, at: Date.now() };
  const machineAskedAt = useRef<{ id: string; at: number } | null>(null);
  if (machine && machineAskedAt.current?.id !== machine) machineAskedAt.current = { id: machine, at: Date.now() };
  useEffect(() => {
    if (list.value === null || list.at === null) return;
    const read = list.at;
    const due = (asked: { id: string; at: number } | null, id: string | null) => id !== null && asked?.id === id && read >= asked.at;
    const sessionDue = due(askedAt.current, session);
    if (sessionDue && session) askedAt.current = { id: session, at: Number.POSITIVE_INFINITY };
    const sessionGone = sessionDue && !list.value.some((r) => r.session_id === session);
    const machineGone = due(machineAskedAt.current, machine) && machine !== null && !listsMachine(list.value, machine);
    if (machineGone) machineAskedAt.current = null;
    if (sessionGone) {
      const said = hosted ? OFF_MACHINE : 'That session is no longer listed.';
      raise(notice(machineGone ? `${said} ${MACHINE_UNLISTED}` : said), 'list');
      void navigate({ search: withMachine('', machineGone ? null : machine) }, { replace: true });
    } else if (machineGone) {
      raise(notice(MACHINE_UNLISTED), view);
      void navigate({ search: withMachine(location.search, null) }, { replace: true, state: location.state });
    }
  }, [session, machine, view, list.value, list.at, hosted, location.search, location.state, navigate, raise]);

  // ---------- where the list was ----------
  const listScroller = useRef<HTMLDivElement>(null);
  const listTop = useRef(0);
  const goSession = useCallback(
    (id: string) => {
      listTop.current = listScroller.current?.scrollTop ?? 0;
      void navigate({ search: withMachine(`?session=${encodeURIComponent(id)}`, machine) });
    },
    [navigate, machine],
  );
  // Back on the list, it is put back where the reader left it, once its rows are there (SE-C195).
  useLayoutEffect(() => {
    if (session === null && rows.length && listScroller.current) listScroller.current.scrollTop = listTop.current;
  }, [session, rows.length]);
  // Another tab is read from its top, not from where the last one was left (SE-C230).
  const lastTab = useRef(machine);
  useLayoutEffect(() => {
    if (lastTab.current === machine) return;
    lastTab.current = machine;
    listTop.current = 0;
    if (listScroller.current) listScroller.current.scrollTop = 0;
  }, [machine]);
  // A session just started is opened, the list read again so its row comes in (SE-C241).
  const onStarted = useCallback(
    (id: string) => {
      void client.invalidateQueries({ queryKey: liveKeys.sessions });
      goSession(id);
    },
    [client, goSession],
  );
  // A tab is a history entry of its own, as a session is (SE-C189) and the Overview's scope (OV-C30);
  // the rest of the address stays, so a session open beside the list stays open (SE-C228).
  const onTab = useCallback((next: string | null) => void navigate({ search: withMachine(location.search, next) }), [navigate, location.search]);

  // ---------- archive and restore ----------
  const [archiving, setArchiving] = useState<ReadonlySet<string>>(() => new Set());
  const [rowNotices, setRowNotices] = useState<ReadonlyMap<string, Notice>>(() => new Map());
  const archive = useCallback(
    async (id: string, archived: boolean): Promise<Outcome<unknown>> => {
      setArchiving((s) => new Set(s).add(id));
      const r = await postLive('/api/live/archive', { session: id, archived });
      setArchiving((s) => {
        const next = new Set(s);
        next.delete(id);
        return next;
      });
      // The list is read again where it is, keeping the reader's place (SE-C34, SE-C40).
      if (r.ok) await client.invalidateQueries({ queryKey: liveKeys.sessions });
      return r;
    },
    [client],
  );
  const actions: RowActions = {
    open: goSession,
    archive: (id, archived) => {
      setRowNotices((m) => {
        const next = new Map(m);
        next.delete(id);
        return next;
      });
      void archive(id, archived).then((r) => {
        if (!r.ok) setRowNotices((m) => new Map(m).set(id, notice(failed('archive', r)) ?? { text: '', ok: false, ms: null }));
      });
    },
    busy: archiving,
    notices: rowNotices,
    dismiss: (id) =>
      setRowNotices((m) => {
        const next = new Map(m);
        next.delete(id);
        return next;
      }),
    openId: session,
  };

  // ---------- Find ----------
  const findSearch = useCallback(
    (q: string, s: FindScope) => {
      const p = new URLSearchParams();
      if (session) p.set('session', session);
      p.set('find', q);
      p.set('scope', s);
      return withMachine(`?${p}`, machine);
    },
    [session, machine],
  );
  const openFind = useCallback(() => {
    void navigate({ search: findSearch(query, session ? 'session' : 'all') }, { state: { find: true } });
  }, [navigate, findSearch, query, session]);
  const closeFind = useCallback(() => {
    // Opened here, it is closed by going back, so Back and this agree; arrived at by its address,
    // it is replaced by the view under it (SE-C175).
    if ((location.state as { find?: boolean } | null)?.find) void navigate(-1);
    else void navigate({ search: withMachine(session ? `?session=${encodeURIComponent(session)}` : '', machine) }, { replace: true });
  }, [location.state, navigate, session, machine]);
  const onQuery = useCallback(
    (q: string) => void navigate({ search: findSearch(q, scope) }, { replace: true, state: location.state }),
    [navigate, findSearch, scope, location.state],
  );
  const onScope = useCallback(
    (s: FindScope) => void navigate({ search: findSearch(query, s) }, { replace: true, state: location.state }),
    [navigate, findSearch, query, location.state],
  );
  const [hitNotices, setHitNotices] = useState<ReadonlyMap<string, Notice>>(() => new Map());
  const onHit = useCallback(
    (h: FindHit, raw: SearchHit) => {
      // Looked up in the router's list; a session it does not list says why, by the hit (SE-C173).
      const target = hitTarget(raw.session_id, list.value !== null ? { rows: list.value } : { failed: refusalText(list.error) }, hosted);
      if ('say' in target) {
        setHitNotices((m) => new Map(m).set(h.key, notice(target.say) ?? { text: target.say, ok: false, ms: null }));
        return;
      }
      const p = new URLSearchParams({ session: raw.session_id, block: String(raw.block_id), ordinal: String(raw.ordinal ?? 0) });
      listTop.current = listScroller.current?.scrollTop ?? 0;
      void navigate({ search: withMachine(`?${p}`, machine) });
    },
    [list.value, list.error, hosted, navigate, machine],
  );

  // ---------- the list's header ----------
  const failedRead = list.stale || list.unreachable ? refusalText(list.error) : null;
  // Until the list has answered, the header has nothing to count: never "nothing yet" (SH-C10).
  const pip =
    list.value === null && failedRead === null
      ? { tone: 'stale' as const, text: 'reading the sessions…', stop: false }
      : header({ feed: null, list: failedRead !== null ? { failed: failedRead } : { rows } }, Date.now());
  const listPrimary = session === null && !findOpen;

  return (
    <KeepContext.Provider value={keep}>
      <div ref={root} className={styles.screen} data-sessions-screen="" data-open={session !== null || findOpen ? '' : undefined}>
        <section className={styles.listPane} aria-label="Sessions by project" data-pane="list">
          <header className={styles.paneHead}>
            {listPrimary ? <h1 className={styles.heading}>Live sessions</h1> : <h2 className={styles.heading}>Live sessions</h2>}
            <StatusPill className={styles.pill} tone={pip.tone === 'stale' ? null : pip.tone} text={pip.text} stale={failedRead !== null || pip.tone === 'stale'} />
            <Toggletip align="bottom-end" className={styles.help}>
              <ToggletipButton label="What live and view mean">
                <Information />
              </ToggletipButton>
              <ToggletipContent>
                <p>
                  <strong>live</strong>: under the wrapper, so it takes a typed message and a Stop. <strong>view</strong>: read-only until it is started
                  again after installing the wrapper.
                </p>
                <p>
                  A Kimi Code session is <strong>live</strong> when this router hosts it, as one started here is, and <strong>view</strong> when it
                  runs in its own terminal or in VS Code.
                </p>
              </ToggletipContent>
            </Toggletip>
            {session === null && (
              <Button kind="ghost" size="md" className={styles.findButton} renderIcon={Search} aria-pressed={findOpen} onClick={findOpen ? closeFind : openFind}>
                Find
              </Button>
            )}
            {/* Where a session is started from: a Kimi Code session the router hosts (SE-C239). */}
            <StartSession hosted={hostedRead.data ?? null} onStarted={onStarted} />
          </header>
          {list.value !== null && failedRead !== null && (
            // Kept, never blanked, and marked with its age (SE-C41).
            <p className={styles.stale}>
              Showing what was read {since(list.at)}: the last read failed — {failedRead}
            </p>
          )}
          <div ref={listScroller} className={styles.listScroll}>
            {listSaidFor === view && <NoticeLine notice={listSaid} onClose={() => raise(null, null)} />}
            {list.value === null ? (
              list.unreachable ? (
                // A read that failed is not an empty list (SH-C22).
                <EmptyState title="Could not read the sessions">{failedRead}</EmptyState>
              ) : (
                <p className={styles.muted}>Reading the sessions…</p>
              )
            ) : (
              <List
                machines={shown}
                hosted={hosted}
                wrapperWarning={wrapperWarning(shim.data, hosted)}
                actions={actions}
                level={listPrimary ? 2 : 3}
                tabs={tabs}
                tab={machine}
                onTab={onTab}
              />
            )}
          </div>
        </section>
        <section className={styles.mainPane} aria-label={findOpen ? 'Find' : 'The open session'} data-pane="main">
          {session ? (
            <SessionView
              key={session}
              id={session}
              row={row}
              rows={rows}
              covered={findOpen}
              arrival={arrival}
              primary={!findOpen}
              onBack={() => void navigate({ search: withMachine('', machine) })}
              onFind={openFind}
              archive={archive}
              archiving={archiving}
            />
          ) : (
            !findOpen && (
              <div className={styles.idle}>
                <EmptyState title="No session open">Choose a session from the list to follow it here.</EmptyState>
              </div>
            )
          )}
          {findOpen && (
            <>
              <header className={styles.paneHead}>
                <h1 className={styles.heading}>Find</h1>
              </header>
              <Find
                key={`${session ?? ''}`}
                openId={session}
                query={query}
                scope={scope}
                onQuery={onQuery}
                onScope={onScope}
                onClose={closeFind}
                onHit={onHit}
                notices={hitNotices}
                dismiss={(key) =>
                  setHitNotices((m) => {
                    const next = new Map(m);
                    next.delete(key);
                    return next;
                  })
                }
              />
            </>
          )}
        </section>
      </div>
    </KeepContext.Provider>
  );
}
