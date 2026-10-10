// New for agent-router: Search the transcripts (assets/index.html, `#searchpanel`, `runSearch`), the Search
// tab. Not part of the five-second poll: it answers the keyboard, not the clock (OV-C42). It stays mounted
// while another tab is open, so what was asked survives a tab change and "/" works from any tab (OV-C37).
import { ContentSwitcher, Search as CarbonSearch, Switch } from '@carbon/react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { BarList } from '~/components/BarList';
import { getJson } from '~/data/http';
import { refusalText } from '~/lib/errors';
import { useCoarse } from '~/lib/viewport';
import { count } from '~/model/format';
import type { Scope } from '~/model/scope';
import { SEARCH_DEBOUNCE_MS, searchFailed, searchIdle, searchParams, searchView, type SearchKind, type SearchReply } from '~/model/search';
import { Panel } from './parts';
import styles from './Overview.module.scss';

/// The five kinds, always visible, all by default (OV-C38).
const KINDS: readonly { readonly kind: SearchKind; readonly label: string }[] = [
  { kind: '', label: 'all' },
  { kind: 'text', label: 'messages' },
  { kind: 'tool_use', label: 'tool calls' },
  { kind: 'tool_result', label: 'results' },
  { kind: 'thinking', label: 'thinking' },
];

interface Asked {
  readonly text: string;
  /// What the router is asked: the text verbatim, trimmed, with the page's scope when it was asked —
  /// its provider, account, project, session and range (OV-C43, OV-C44, OV-C227); null for a blank
  /// query (OV-C40).
  readonly params: Record<string, string> | null;
  /// Each ask is its own read: the router does not cache a search, and nor does the page (OV-C42).
  readonly seq: number;
}

/// Whether focus is somewhere a typed slash belongs to (OV-C37).
const typing = (el: Element | null): boolean => !!el && (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || (el as HTMLElement).isContentEditable);

export function SearchPanel({
  scope,
  onScope,
  now,
  shown,
  onShow,
}: {
  scope: Scope;
  onScope: (next: Scope) => void;
  now: number;
  /// Its tab is the open one.
  shown: boolean;
  /// Open its tab.
  onShow: () => void;
}) {
  const coarse = useCoarse();
  const input = useRef<HTMLInputElement>(null);
  const [text, setText] = useState('');
  const [kind, setKind] = useState<SearchKind>('');
  const [asked, setAsked] = useState<Asked | null>(null);
  const seq = useRef(0);
  const scopeNow = useRef(scope);
  scopeNow.current = scope;

  const ask = (t: string, k: SearchKind) => {
    seq.current += 1;
    setAsked({ text: t, params: searchParams(t, k, scopeNow.current, Date.now()), seq: seq.current });
  };

  // Typing searches 160 ms after the last keystroke (OV-C39).
  const typed = useRef(false);
  useEffect(() => {
    if (!typed.current) return;
    const t = setTimeout(() => ask(text, kind), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(t);
    // The kind is read when the pause ends; changing it asks at once, below.
  }, [text]);

  // "/" opens the Search tab, then focuses the box and selects its text, from anywhere except in a
  // field or with a modifier (OV-C37). A hidden box takes no focus, so from another tab the focus
  // waits for the tab to open.
  const focusWanted = useRef(false);
  const showNow = useRef(onShow);
  showNow.current = onShow;
  const shownNow = useRef(shown);
  shownNow.current = shown;
  const focusBox = () => {
    input.current?.focus();
    input.current?.select();
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
      if (typing(document.activeElement)) return;
      e.preventDefault();
      if (shownNow.current) return focusBox();
      focusWanted.current = true;
      showNow.current();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
  useEffect(() => {
    if (!shown || !focusWanted.current) return;
    focusWanted.current = false;
    focusBox();
  });

  const params = asked?.params ?? null;
  const q = useQuery({
    queryKey: ['overview', 'search', asked?.seq ?? 0, params] as const,
    queryFn: async ({ signal }) => {
      const started = performance.now();
      const reply = await getJson<SearchReply>(`/api/search?${new URLSearchParams(params ?? {})}`, { signal });
      return { reply, tookMs: performance.now() - started, text: asked?.text ?? '' };
    },
    // A blank query is not a search: the results are put away (OV-C40).
    enabled: params !== null,
    staleTime: Infinity,
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    // Earlier results stay while a later query is read; a reply to an earlier query, arriving after
    // a later one was asked, lands under a key nobody reads (OV-C41).
    placeholderData: keepPreviousData,
  });

  const blank = params === null;
  const view = !blank && q.data ? searchView(q.data.reply, q.data.text, scope, q.data.tookMs, now) : null;
  const failed = !blank && q.isError ? searchFailed(refusalText(q.error)) : null;
  const caption = blank ? searchIdle(scope) : failed ? 'search failed' : view ? view.caption : 'searching…';

  return (
    <Panel concept="search" title="Search the transcripts" caption={caption}>
      <div className={styles.searchBar}>
        <div className={styles.searchField}>
          <CarbonSearch
            ref={input}
            size={coarse ? 'lg' : 'md'}
            labelText="Search the transcripts"
            placeholder="a word, a phrase, an identifier…"
            closeButtonLabelText="Clear the search"
            value={text}
            onChange={(e) => {
              typed.current = true;
              setText(e.target.value);
            }}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
          <kbd className={styles.kbd} aria-hidden="true">
            /
          </kbd>
        </div>
        <ContentSwitcher
          className={styles.kinds}
          aria-label="What to search"
          size={coarse ? 'lg' : 'md'}
          selectedIndex={Math.max(0, KINDS.findIndex((k) => k.kind === kind))}
          onChange={({ index }: { index?: number }) => {
            const next = KINDS[index ?? 0]?.kind ?? '';
            if (next === kind) return;
            setKind(next);
            // Changing the kind searches at once (OV-C39).
            ask(text, next);
          }}
        >
          {KINDS.map((k) => (
            <Switch key={k.label} name={k.kind || 'all'} text={k.label} />
          ))}
        </ContentSwitcher>
      </div>
      {!blank && (failed || view) && (
        <div className={styles.body} data-search-results="">
          {failed ? (
            <p className={styles.problem} role="status">
              {failed}
            </p>
          ) : view ? (
            <>
              {view.sessions.length > 0 && (
                <>
                  <p className={styles.subhead}>{view.sessionsHeading}</p>
                  <BarList
                    label="Sessions that mention it"
                    rows={view.sessions.map((s) => ({
                      key: s.id,
                      name: s.name,
                      mono: true,
                      concept: 'session',
                      path: s.project || '(unresolved)',
                      chips: <code className={styles.mono}>{s.id}</code>,
                      value: s.hits,
                      figure: count(s.hits),
                      meta: s.meta,
                    }))}
                    selected={scope.session}
                    onSelect={(key) => {
                      const row = view.sessions.find((s) => s.id === key);
                      if (row) onScope(row.scope);
                    }}
                  />
                </>
              )}
              {view.hits.length > 0 ? (
                <>
                  <p className={styles.subhead}>{view.hitsHeading}</p>
                  <ul className={styles.hits} aria-label="Best matches">
                    {view.hits.map((h) => (
                      <li key={h.key}>
                        <button type="button" className={styles.hit} onClick={() => onScope(h.scope)}>
                          <span className={styles.hitHead}>
                            <span className={styles.kindTag}>{h.kind}</span> <span className={styles.role}>{h.role}</span> <span>· {h.project}</span>{' '}
                            <span className={styles.mono}>· {h.session}</span> <span>· {h.at}</span>
                          </span>
                          <span className={styles.snippet}>
                            {h.snippet.map((run, i) => (run.mark ? <mark key={i}>{run.text}</mark> : <span key={i}>{run.text}</span>))}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </>
              ) : (
                <p className={styles.empty}>{view.empty}</p>
              )}
            </>
          ) : null}
        </div>
      )}
    </Panel>
  );
}
