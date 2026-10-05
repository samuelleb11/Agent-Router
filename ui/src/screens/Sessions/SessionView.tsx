// New for agent-router: one open session — its header, tabs, facts, feed, prompt and composer — the
// live view's `openSession` (assets/live.html). The screen mounts it keyed on the session's id
// (SE-C192), so every piece of state here belongs to that session, and a result that comes back
// once another session is on screen lands in a component that is gone (SE-C193).
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, InlineNotification } from '@carbon/react';
import { ArrowLeft, Search } from '@carbon/icons-react';
import { StatusPill } from '~/components/StatusDot';
import { liveKeys, postLive, readPrompt, type Outcome } from '~/data/live';
import { id8 } from '~/model/format';
import type { SearchHit } from '~/model/search';
import {
  notice,
  permissionCard,
  projectOf,
  sendNotice,
  spoken,
  tabsFor,
  vouched,
  type FeedEntry,
  type Pip,
  type SessionRow,
} from '~/model/session';
import { Composer } from './Composer';
import { Facts } from './Facts';
import { Feed } from './Feed';
import { entryFor, failed, hitIn, type Arrival } from './logic';
import { NoticeLine } from './Notice';
import { useNotice } from './useNotice';
import { PermissionCard, type Answered } from './PermissionCard';
import type { Deliver } from './QuestionForm';
import { Strip } from './Strip';
import { useSessionStream } from './useSessionStream';
import styles from './Sessions.module.scss';

/// The prompt is asked for every two seconds while the feed is on screen (SE-C120).
export const PROMPT_MS = 2000;

/// Within this of the bottom, the reader is following the newest row (SE-C87).
const FOLLOW_PX = 140;

const CONNECTING: Pip = { tone: 'stale', text: 'connecting…', stop: false };

export interface SessionViewProps {
  readonly id: string;
  /// The session's row as the list last had it; null until the list has been read.
  readonly row: SessionRow | null;
  /// Every row the list last had, for the tabs.
  readonly rows: readonly SessionRow[];
  /// Find lies over the session: nothing of it is drawn, and its stream stays connected (SE-C174).
  readonly covered: boolean;
  /// The entry the address arrived at, if any.
  readonly arrival: Arrival | null;
  /// Whether this pane's heading is the page's first.
  readonly primary: boolean;
  readonly onBack: () => void;
  readonly onFind: () => void;
  readonly archive: (id: string, archived: boolean) => Promise<Outcome<unknown>>;
  readonly archiving: ReadonlySet<string>;
}

export function SessionView({ id, row, rows, covered, arrival, primary, onBack, onFind, archive, archiving }: SessionViewProps) {
  const stream = useSessionStream(id);
  const { feed, echo, aside } = stream;
  const header = stream.header ?? CONNECTING;

  // ---------- following the newest row (SE-C87, SE-C88) ----------
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const following = useRef(arrival === null);
  const pin = useCallback(() => {
    const el = scroller.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    // Once more after layout settles: a row folds a frame after it lands.
    requestAnimationFrame(() => {
      const s = scroller.current;
      if (following.current && s) s.scrollTop = s.scrollHeight;
    });
  }, []);
  const follow = useCallback(() => {
    following.current = true;
    pin();
  }, [pin]);
  const onScroll = () => {
    const el = scroller.current;
    if (el) following.current = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_PX;
  };
  // The scroller never resizes; the rows inside it do, so they are what is watched.
  useEffect(() => {
    const el = content.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const watch = new ResizeObserver(() => {
      if (following.current) pin();
    });
    watch.observe(el);
    return () => watch.disconnect();
  }, [pin, covered]);
  // A visual viewport that shrinks under the keyboard keeps the newest message in view (SE-C200).
  useEffect(() => {
    const vv = globalThis.visualViewport;
    if (!vv) return;
    const keep = () => {
      if (following.current) pin();
    };
    vv.addEventListener('resize', keep);
    return () => vv.removeEventListener('resize', keep);
  }, [pin]);

  const entries = feed?.entries ?? EMPTY;
  // A rebuilt feed — a fresh connection's replay — is followed from its top row down (SE-C88).
  const first = useRef<FeedEntry | undefined>(undefined);
  useLayoutEffect(() => {
    const rebuilt = first.current !== undefined && entries[0] !== first.current;
    first.current = entries[0];
    if (rebuilt && arrival === null) following.current = true;
    if (following.current) pin();
  }, [entries, arrival, pin]);
  // Closing Find comes back to the newest row (SE-C174).
  useLayoutEffect(() => {
    if (!covered && arrival === null) follow();
  }, [covered, arrival, follow]);

  // ---------- each entry's key, by its identity ----------
  const keyMap = useRef(new WeakMap<FeedEntry, string>());
  const counter = useRef(0);
  const keys = useCallback((e: FeedEntry) => {
    let k = keyMap.current.get(e);
    if (!k) {
      counter.current += 1;
      k = `e${counter.current}`;
      keyMap.current.set(e, k);
    }
    return k;
  }, []);

  // ---------- arriving at an entry ----------
  // The hit is looked for among the searches this page holds until it is found, and then kept: a
  // search the cache lets go of later does not take the mark with it.
  const client = useQueryClient();
  const found = useRef<SearchHit | null>(null);
  if (arrival && !found.current) found.current = hitIn(client.getQueryCache().getAll().map((q) => q.state.data), id, arrival);
  const hit = arrival ? found.current : null;
  const arrivedAt = hit ? entryFor(entries, hit) : null;
  const arrivedEl = useRef<HTMLDivElement>(null);
  const landed = useRef(false);
  useLayoutEffect(() => {
    if (arrivedAt === null || landed.current || covered) return;
    // Scrolled to, focused once, and kept marked for as long as it is there.
    landed.current = true;
    const el = arrivedEl.current;
    el?.scrollIntoView?.({ block: 'center' });
    el?.focus({ preventScroll: true });
  }, [arrivedAt, covered]);
  // Once the replay is in and the entry is not, the feed says so, and follows the newest row.
  const missed = arrival !== null && !landed.current && arrivedAt === null && feed?.phase === 'live';
  useEffect(() => {
    if (missed) follow();
  }, [missed, follow]);

  // ---------- the tabs ----------
  const now = Date.now();
  const tabs = row ? tabsFor(row, rows, vouched(feed), now) : [];
  const [stripNotice, sayStrip] = useNotice();
  const archiveTab = async (tab: string) => {
    const r = await archive(tab, true);
    sayStrip(r.ok ? null : notice(failed('archive', r)));
  };

  // ---------- the prompt and its card ----------
  const prompt = useQuery({
    queryKey: liveKeys.prompt(id),
    queryFn: ({ signal }) => readPrompt(id, signal),
    refetchInterval: PROMPT_MS,
    enabled: !covered,
    retry: false,
    staleTime: 0,
    gcTime: 0,
    refetchOnWindowFocus: false,
  });
  const cue = feed?.cue ?? null;
  const refetchPrompt = prompt.refetch;
  // A status naming a waiting request the screen has not asked about is asked about at once.
  useEffect(() => {
    if (cue && !covered) void refetchPrompt();
  }, [cue, covered, refetchPrompt]);
  const [answered, setAnswered] = useState<ReadonlySet<string>>(() => new Set());
  const [cardNotice, sayCard] = useNotice();
  const card = covered ? null : permissionCard(prompt.data, answered);
  const onAnswered = (a: Answered) => {
    setAnswered((s) => new Set(s).add(a.requestId));
    sayCard(a.notice);
    if (a.kind === 'gone') void refetchPrompt();
  };

  // ---------- sending ----------
  const deliver: Deliver = useCallback(
    async (text, opts = {}) => {
      const r = await postLive<Record<string, unknown>>('/api/live/send', { session: id, text });
      if (!r.ok) return { ok: false, notice: notice(failed('send', r)) };
      echo(text);
      follow();
      return { ok: true, notice: sendNotice(200, r.body, opts) };
    },
    [id, echo, follow],
  );

  // What the header says, announced only when what the session is doing changes, never at each
  // heartbeat whose duration ticks (SE-C206).
  const said = vouched(feed);
  const announced = said ? said.detail || spoken(said) : header.text;
  const Heading = primary ? 'h1' : 'h2';

  if (covered) return null;
  return (
    <div className={styles.session} data-session-view={id}>
      <header className={styles.paneHead} data-session-head="">
        <Button kind="ghost" size="md" className={styles.back} hasIconOnly renderIcon={ArrowLeft} iconDescription="Back to the list" tooltipPosition="bottom" onClick={onBack} />
        <Heading className={styles.heading}>{row ? projectOf(row) : id8(id)}</Heading>
        <StatusPill className={styles.pill} tone={header.tone === 'stale' ? null : header.tone} text={header.text} stale={header.tone === 'stale'} />
        <span className={styles.announce} aria-live="polite">
          {announced}
        </span>
        <Button kind="ghost" size="md" className={styles.findButton} renderIcon={Search} onClick={onFind}>
          Find
        </Button>
      </header>
      {tabs.length >= 2 && <Strip tabs={tabs} onArchive={(t) => void archiveTab(t)} busy={archiving} notice={stripNotice} onNoticeClose={() => sayStrip(null)} />}
      <Facts session={id} row={row} />
      <div ref={scroller} className={styles.feedScroll} onScroll={onScroll} data-feed-scroller="">
        <div ref={content}>
          {arrival !== null && missed && (
            <InlineNotification
              className={styles.notice}
              kind="info"
              lowContrast
              hideCloseButton
              subtitle={
                hit
                  ? 'This feed shows the transcript’s tail, and the entry this address names is earlier than it reaches. Find searches all of this session.'
                  : 'This address names an entry from a search this tab no longer holds, so it cannot be placed in the feed. Find searches all of this session.'
              }
            />
          )}
          <Feed session={id} entries={entries} deliver={deliver} keys={keys} arrived={arrivedAt} arrivedRef={arrivedEl} />
        </div>
      </div>
      <div className={styles.dock} data-dock="">
        {/* Above the composer: nothing typed below runs until it is answered (SE-C122). A tall
            card scrolls inside its own area, so the feed above keeps its room. */}
        {(card || cardNotice) && (
          <div className={styles.cardArea}>
            {card && <PermissionCard key={card.seen} session={id} card={card} onAnswered={onAnswered} />}
            <NoticeLine notice={cardNotice} onClose={() => sayCard(null)} />
          </div>
        )}
        <Composer session={id} sent={feed?.sent ?? EMPTY_SENT} stoppable={header.stop} deliver={deliver} echo={echo} aside={aside} follow={follow} />
      </div>
    </div>
  );
}

const EMPTY: readonly FeedEntry[] = [];
const EMPTY_SENT: readonly string[] = [];
