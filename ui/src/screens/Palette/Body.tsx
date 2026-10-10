// Copied from Northstar ui/src/screens/Palette/Palette.tsx at e9c1589: its Body and the parts that draw it.
// Changed: in its own module, loaded the first time the palette opens, so the first load of every page does not carry it (scripts/check-chunks.mjs); the sources are the router's (its sessions, its projects, its search index, its accounts and what a session can be switched to), read only while the palette is open and the accounts only while the commands are on screen; typing searches after a pause and shows the latest keystroke's answer only; a search's hits run by kind under a switcher of the kinds the Overview offers, each with its snippet; a hit can be a write to the router, run here with its refusal beside the list in the router's words, or a step that asks which session or which value first; each row is named for a screen reader from its facts; a Close button, because a phone has no Escape key. Northstar's reference, item and project-list states and its population line are gone with the items.
import { useEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Button, ContentSwitcher, InlineLoading, InlineNotification, Search, SkeletonText, Switch } from '@carbon/react';
import { notify } from '~/app/notify';
import { useTheme } from '~/app/theme';
import { EmptyState } from '~/components/EmptyState';
import { Refusal } from '~/components/Refusal';
import { availabilityOf, useRead, type Read } from '~/data/held';
import { postJson } from '~/data/http';
import { useProjects } from '~/data/projects';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { routes } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import {
  SEARCH_SESSIONS_CAP,
  paletteKeys,
  readAccounts,
  readLiveOptions,
  readLiveSessions,
  readSearch,
  type SearchKind,
} from './api';
import {
  COMMAND_PREFIX,
  SEARCH_DEBOUNCE_MS,
  SEARCH_KINDS,
  SEARCH_LIMIT,
  describeStep,
  kindRun,
  markRuns,
  resolve,
  sessionName,
  shortId,
  type Context,
  type Hit,
  type Mode,
  type Notice,
  type Post,
  type SearchState,
  type Source,
  type Step,
} from './hits';
import { INPUT_ID } from './ids';
import frame from './Palette.module.scss';
import styles from './Body.module.scss';

const LIST_ID = 'palette-results';
const hitId = (i: number) => `palette-hit-${i}`;

/// A value that follows `value` once it has stopped changing for `ms`.
function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/// The router's search, asked once the typing pauses (SH-C52, OV-C39) and at once when the kind
/// changes. Each query is its own cache entry and the view reads only the one for what is typed
/// now, so an answer to an earlier keystroke that lands late is never shown. Not polled, and
/// never retried quietly: it answers the keyboard, not the clock (OV-C42).
function useSearch(text: string, kind: SearchKind | null): SearchState {
  const asked = useSettled(text, SEARCH_DEBOUNCE_MS);
  const q = useQuery({
    queryKey: paletteKeys.search(asked, kind),
    queryFn: ({ signal }) => readSearch(asked, kind, SEARCH_LIMIT, signal),
    enabled: asked !== '',
    retry: false,
    refetchOnWindowFocus: false,
  });
  if (text === '') return { status: 'idle' };
  if (asked !== text) return { status: 'waiting' };
  if (q.status === 'error') return { status: 'failed', error: q.error };
  if (q.status === 'success') return { status: 'read', answer: q.data };
  return { status: 'waiting' };
}

function sourceOf<T>(read: Read<T>, empty: T): Source<T> {
  return { state: availabilityOf(read), value: read.value ?? empty, ...(read.unreachable ? { error: read.error } : {}) };
}

/// The session open on the Sessions screen, which lives in its address (SE-C189).
function useSessionOnScreen(): string | null {
  const location = useLocation();
  if (location.pathname !== routes.sessions) return null;
  return new URLSearchParams(location.search).get('session') || null;
}

/// What the palette holds while it is open. Mounted only then, so the typing, the step and the
/// cursor start clean each time and nothing is read while nobody is looking.
export function Body({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const theme = useTheme();
  const coarse = useCoarse();
  const onScreen = useSessionOnScreen();
  const [query, setQuery] = useState('');
  const [cursor, setCursor] = useState(0);
  const [kind, setKind] = useState<SearchKind | null>(null);
  const [steps, setSteps] = useState<readonly Step[]>([]);
  const [refused, setRefused] = useState<{ readonly title: string; readonly error: unknown } | null>(null);
  const [doing, setDoing] = useState<string | null>(null);
  // Whether the palette is still on screen when a write answers: a refusal that comes back after
  // it closed has no control to stand beside, so it is said as a notice instead.
  const shown = useRef(true);
  useEffect(() => {
    shown.current = true;
    return () => {
      shown.current = false;
    };
  }, []);

  const step = steps.length > 0 ? steps[steps.length - 1]! : null;
  const raw = query.trim();
  const mode: Mode = step ? 'step' : raw.startsWith(COMMAND_PREFIX) ? 'command' : raw ? 'text' : 'empty';
  const searchable = mode === 'empty' || mode === 'text';

  // The rail's own read of the projects, under its key: one read, not two.
  const projects = useProjects();
  const sessions = useRead({ queryKey: paletteKeys.sessions, queryFn: ({ signal }) => readLiveSessions(signal) });
  const accounts = useRead({ queryKey: paletteKeys.accounts, queryFn: ({ signal }) => readAccounts(signal), enabled: mode === 'command' });
  const options = useRead({
    queryKey: paletteKeys.options,
    queryFn: ({ signal }) => readLiveOptions(signal),
    enabled: step?.kind === 'value',
  });
  const search = useSearch(mode === 'text' ? raw : '', kind);

  const ctx: Context = {
    step,
    projects: sourceOf(projects, []),
    sessions: sourceOf(sessions, []),
    accounts: sourceOf(accounts, []),
    options: sourceOf(options, null),
    onScreen,
    theme: theme.choice,
    search,
  };
  const result = resolve(query, ctx);
  // The runs as the resolver ordered them, each row keeping its flat index, which is what the
  // keyboard and `aria-activedescendant` address.
  const runs = result.runs.reduce<{ key: string; label: string; of?: number; rows: { hit: Hit; index: number }[] }[]>((out, run) => {
    const start = out.reduce((n, r) => n + r.rows.length, 0);
    out.push({ key: run.key, label: run.label, of: run.of, rows: run.hits.map((hit, i) => ({ hit, index: start + i })) });
    return out;
  }, []);
  const hits = runs.flatMap((r) => r.rows.map((x) => x.hit));

  // The cursor is clamped at render rather than reset in an effect: the list can shrink under
  // it when a read lands, and it must still point at a row.
  const active = hits.length === 0 ? -1 : Math.min(cursor, hits.length - 1);

  function back() {
    setSteps(steps.slice(0, -1));
    setQuery('');
    setCursor(0);
    setRefused(null);
  }

  async function run(post: Post) {
    setDoing(post.doing);
    setRefused(null);
    try {
      const answer = await postJson<unknown>(post.path, post.body);
      // Whatever the write changed is read again wherever it is shown: the account that signs,
      // a session's shelf, its settings (ST-C24).
      void client.invalidateQueries();
      notify.saved(post.done(answer).title);
      setDoing(null);
      onClose();
    } catch (error) {
      // A refusal is read again too: what was refused may be gone or changed since the list
      // was drawn (ST-C25).
      void client.invalidateQueries();
      if (shown.current) {
        setRefused({ title: post.refused, error });
        setDoing(null);
      } else {
        notify.refused(post.refused, refusalText(error));
      }
    }
  }

  function choose(hit: Hit | undefined) {
    if (!hit?.action || doing !== null) return;
    const action = hit.action;
    switch (action.type) {
      case 'go':
        void navigate(action.href);
        onClose();
        return;
      case 'theme':
        theme.set(action.choice);
        onClose();
        return;
      case 'step':
        setSteps([...steps, action.step]);
        setQuery('');
        setCursor(0);
        setRefused(null);
        return;
      case 'back':
        back();
        return;
      case 'post':
        void run(action.post);
        return;
    }
  }

  function onKey(e: KeyboardEvent<HTMLInputElement>) {
    switch (e.key) {
      case 'Escape':
        // Carbon's Search clears the field on Escape and stops the event there, so the modal
        // never hears it; the palette closes instead, which is what Escape means here.
        e.preventDefault();
        e.stopPropagation();
        onClose();
        return;
      case 'Backspace':
        // In an empty field, a step goes back to where it came from.
        if (step && query === '') {
          e.preventDefault();
          back();
        }
        return;
      case 'ArrowDown':
        e.preventDefault();
        setCursor(active < 0 ? 0 : Math.min(active + 1, hits.length - 1));
        return;
      case 'ArrowUp':
        e.preventDefault();
        setCursor(Math.max(active - 1, 0));
        return;
      case 'Home':
        if (hits.length) {
          e.preventDefault();
          setCursor(0);
        }
        return;
      case 'End':
        if (hits.length) {
          e.preventDefault();
          setCursor(hits.length - 1);
        }
        return;
      case 'Enter':
        e.preventDefault();
        choose(hits[active]);
        return;
      default:
        return;
    }
  }

  function onChange(e: ChangeEvent<HTMLInputElement>) {
    setQuery(e.target.value);
    setCursor(0);
    setRefused(null);
  }

  const onScreenName = onScreen === null ? null : nameOf(onScreen, ctx);

  return (
    <div className={frame.body}>
      <div className={styles.field}>
        <Search
          id={INPUT_ID}
          size="lg"
          labelText="Search and commands"
          placeholder={placeholder(step)}
          closeButtonLabelText="Clear"
          autoComplete="off"
          value={query}
          onChange={onChange}
          onKeyDown={onKey}
          aria-controls={LIST_ID}
          aria-activedescendant={active >= 0 ? hitId(active) : undefined}
          aria-autocomplete="list"
          // On a first open this module arrives after the modal looked for its field, so the
          // field takes the focus itself.
          autoFocus
        />
        {/* A phone has no Escape key, and a full-height sheet has no outside to tap. */}
        <Button kind="ghost" size="lg" className={styles.close} onClick={onClose}>
          Close
        </Button>
      </div>

      {step && (
        <div className={styles.step}>
          <Button kind="ghost" size={coarse ? 'lg' : 'sm'} onClick={back}>
            Back
          </Button>
          <p className={styles.stepTitle}>{describeStep(step)}</p>
        </div>
      )}

      {searchable && (
        <div className={styles.kinds}>
          <ContentSwitcher
            aria-label="Search in"
            size={coarse ? 'lg' : 'sm'}
            selectedIndex={Math.max(
              0,
              SEARCH_KINDS.findIndex((k) => k.kind === kind),
            )}
            onChange={({ index }) => {
              setKind(SEARCH_KINDS[index ?? 0]?.kind ?? null);
              setCursor(0);
            }}
          >
            {SEARCH_KINDS.map((k) => (
              <Switch key={k.label} name={k.kind ?? 'all'} text={k.label} />
            ))}
          </ContentSwitcher>
        </div>
      )}

      {/* Everything between the controls and the scope line: whatever stands where the hits
          would, and the hits themselves. This is the one part that scrolls, so the field stays
          under the thumb and the scope line stays in sight. */}
      <div className={styles.results}>
        {doing !== null && <InlineLoading className={styles.doing} description={doing} status="active" />}
        {refused && (
          <div className={styles.refusal}>
            <Refusal error={refused.error} title={refused.title} />
          </div>
        )}
        {result.notices.map((n) => (
          <NoticeRow key={n.title} notice={n} />
        ))}

        {hits.length > 0 && (
          <div id={LIST_ID} role="listbox" aria-label="Results" aria-busy={doing !== null || undefined} className={styles.list}>
            {runs.map((run) => (
              <div key={run.key} role="group" aria-label={run.label} className={styles.group}>
                <p className={styles.groupLabel}>
                  {run.label}
                  {run.of !== undefined && <span className={styles.count}>{` · ${run.rows.length} of ${run.of}`}</span>}
                </p>
                <div className={styles.rows}>
                  {run.rows.map(({ hit: h, index: i }) => (
                    <div
                      key={h.id}
                      id={hitId(i)}
                      role="option"
                      aria-label={spoken(h)}
                      aria-selected={i === active}
                      aria-disabled={h.action === null || undefined}
                      className={`${styles.hit} ${i === active ? styles.active : ''}`}
                      onMouseMove={() => {
                        if (i !== cursor) setCursor(i);
                      }}
                      onClick={() => choose(h)}
                    >
                      <span className={styles.ref}>{h.label}</span>
                      <span className={styles.title}>
                        {h.detail}
                        {h.ident && (
                          <>
                            {h.detail ? ' · ' : ''}
                            <span className={styles.ident}>{h.ident}</span>
                          </>
                        )}
                        {h.note && <span className={styles.note}> · {h.note}</span>}
                      </span>
                      <span className={styles.meta}>{h.at !== undefined && since(h.at)}</span>
                      {h.snippet !== undefined && <Snippet text={h.snippet} />}
                      {h.action === null && h.why && <span className={styles.why}>{h.why}</span>}
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}

        {mode === 'text' && <SearchStatus search={search} query={raw} kind={kind} jumped={hits.some((h) => !h.snippet)} />}
        {mode === 'command' && hits.length === 0 && (
          <div className={styles.empty}>
            <EmptyState title="No command matches">Type {COMMAND_PREFIX} on its own to list every command.</EmptyState>
          </div>
        )}
        {step && hits.length === 0 && result.notices.every((n) => n.tone !== 'reading' && n.tone !== 'unavailable') && (
          <div className={styles.empty}>
            <EmptyState title={raw ? `Nothing matches “${raw}”` : 'Nothing to choose'}>
              {step.kind === 'session' ? 'The router lists no session this applies to.' : 'Clear the field to see every choice.'}
            </EmptyState>
          </div>
        )}
      </div>

      <div className={styles.scope}>
        <p className={styles.lead} aria-live="polite">
          <Scope mode={mode} search={search} kind={kind} onScreen={onScreenName} />
        </p>
      </div>
    </div>
  );
}

/// What the field invites, by where the palette stands.
function placeholder(step: Step | null): string {
  if (step === null) return `Words, a session or a project, or ${COMMAND_PREFIX} for commands`;
  switch (step.kind) {
    case 'session':
      return 'Filter the sessions';
    case 'value':
      return step.setting === 'model' ? 'Filter the models, or type one' : `Filter the ${step.setting === 'mode' ? 'modes' : 'levels'}`;
    case 'bypass':
      return 'Choose below';
  }
}

/// A session's name as the router lists it, else its short id.
function nameOf(id: string, ctx: Context): string {
  const listed = ctx.sessions.value.find((s) => s.session_id === id);
  return listed ? sessionName(listed) : shortId(id);
}

/// What a row is called to a screen reader: its facts in the order they are drawn, with the
/// pauses its columns make for the eye. Read from its content, the spans would run together.
function spoken(h: Hit): string {
  const facts = [h.label, h.detail, h.ident, h.note, h.at === undefined ? undefined : since(h.at)].filter(Boolean).join(', ');
  const snippet = h.snippet === undefined ? '' : `: ${markRuns(h.snippet)
    .map((r) => r.text)
    .join('')}`;
  const why = h.action === null && h.why ? `. ${h.why}` : '';
  return `${facts}${snippet}${why}`;
}

/// The router's snippet, its matches marked by splitting, never by inserting markup (SE-C170).
function Snippet({ text }: { text: string }) {
  return (
    <span className={styles.snippet}>
      {markRuns(text).map((r, i) => (r.marked ? <mark key={i}>{r.text}</mark> : <span key={i}>{r.text}</span>))}
    </span>
  );
}

function NoticeRow({ notice }: { notice: Notice }) {
  switch (notice.tone) {
    case 'hint':
      return <p className={styles.hint}>{notice.text}</p>;
    case 'reading':
      return <Reading what={`the ${notice.title}`} />;
    case 'unavailable':
      return (
        <div className={styles.refusal}>
          <Refusal error={notice.error ?? notice.title} title={notice.title} />
          <p className={styles.hint}>{notice.text}</p>
        </div>
      );
    case 'warning':
      return (
        <InlineNotification kind="warning" lowContrast hideCloseButton title={notice.title} subtitle={notice.text} className={styles.notice} />
      );
  }
}

/// A read in flight is neither empty nor unreachable, and must look like neither.
function Reading({ what }: { what: string }) {
  return (
    <div className={styles.reading} aria-busy="true">
      <p className={styles.hint}>Reading {what}…</p>
      <SkeletonText paragraph lineCount={2} />
    </div>
  );
}

/// What stands where the search's hits would when there are none to list: the search on its way,
/// why it failed, or which empty it is. A failed search is never "nothing matched" (SE-C167), and
/// an empty one always says so (SE-C166).
function SearchStatus({ search, query, kind, jumped }: { search: SearchState; query: string; kind: SearchKind | null; jumped: boolean }) {
  switch (search.status) {
    case 'idle':
      return null;
    case 'waiting':
      return (
        <div className={styles.reading} aria-busy="true">
          <p className={styles.hint}>Searching for “{query}”…</p>
        </div>
      );
    case 'failed':
      return (
        <div className={styles.refusal}>
          <Refusal error={search.error} title="The search failed" />
        </div>
      );
    case 'read':
      if (search.answer.hits.length > 0) return null;
      return (
        <div className={styles.empty}>
          <EmptyState title={`${jumped ? 'No conversation' : 'Nothing'} matched “${query}”`}>
            {kind === null
              ? 'The search reaches every message, tool call and result the router recorded.'
              : `Only ${kindRun(kind).toLowerCase()} were searched; All reaches every kind.`}
          </EmptyState>
        </div>
      );
  }
}

/// What was searched, how much of it is shown, and what a command acts on: the facts that qualify
/// the list above, in one line under it.
function Scope({ mode, search, kind, onScreen }: { mode: Mode; search: SearchState; kind: SearchKind | null; onScreen: string | null }) {
  switch (mode) {
    case 'empty':
      return <>The search reaches every conversation the router recorded.</>;
    case 'command':
      return onScreen === null ? (
        <>No session is on screen, so a session command asks which one first.</>
      ) : (
        <>Session commands act on {onScreen}, the session on screen.</>
      );
    case 'step':
      return <>Backspace in an empty field goes back a step; Escape closes.</>;
    case 'text':
      return <SearchScope search={search} kind={kind} />;
  }
}

/// How many hits the router counted and how many are shown, loud when it is fewer (SE-C165), and
/// what the router ran when it was not what was typed (SE-C168).
function SearchScope({ search, kind }: { search: SearchState; kind: SearchKind | null }) {
  if (search.status === 'waiting') return <>Searching every conversation the router recorded…</>;
  if (search.status === 'failed') return <>The search did not answer; sessions and projects are still matched by name.</>;
  if (search.status !== 'read') return null;
  const { answer } = search;
  const total = answer.sessions.reduce((n, s) => n + s.hits, 0);
  const atLeast = answer.sessions.length >= SEARCH_SESSIONS_CAP ? 'at least ' : '';
  const sessions = answer.sessions.length;
  const only = kind === null ? '' : ` Only ${kindRun(kind).toLowerCase()}.`;
  const ran = answer.expression && answer.expression !== answer.query ? ` Searched for ${answer.expression}.` : '';
  if (total > answer.hits.length) {
    return (
      <>
        Showing the {answer.hits.length} best of {atLeast}
        {total} matches, in {sessions} {sessions === 1 ? 'session' : 'sessions'}; type more to narrow.{only}
        {ran}
      </>
    );
  }
  return (
    <>
      {total} {total === 1 ? 'match' : 'matches'} in {sessions} {sessions === 1 ? 'session' : 'sessions'}.{only}
      {ran}
    </>
  );
}
