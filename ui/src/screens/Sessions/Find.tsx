// New for agent-router: the live view's Find (assets/live.html, `openFind`, `runFind`, `drawFind`)
// on Carbon. The feed only ever shows the transcript's tail; the router's index holds all of it,
// so from a session Find is the only way to its earlier turns (SE-C161). It never touches the
// stream: the session under it stays connected (SE-C174).
import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button, ContentSwitcher, Search, Switch } from '@carbon/react';
import { Close } from '@carbon/icons-react';
import { liveKeys, readFind } from '~/data/live';
import { Refused, refusalText } from '~/lib/errors';
import { findParams, findPlaceholder, findScopes, findView, FIND_DEBOUNCE_MS, type FindHit, type FindResult, type FindScope, type SearchHit } from '~/model/search';
import { NoticeLine } from './Notice';
import type { Notice } from '~/model/session';
import styles from './Sessions.module.scss';

/// A value that follows `value` once it has stopped changing for `ms`.
function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/// Find over the open session or over everything. Its query and scope are the address's (SE-C175),
/// so Back closes it and a reload keeps it; typing replaces the address rather than adding to the
/// history (SH-C53), and searches 200 ms after the last keystroke, an older query's reply never
/// replacing a newer one's (SE-C164).
export function Find({
  openId,
  query,
  scope,
  onQuery,
  onScope,
  onClose,
  onHit,
  notices,
  dismiss,
}: {
  openId: string | null;
  query: string;
  scope: FindScope;
  onQuery: (q: string) => void;
  onScope: (s: FindScope) => void;
  onClose: () => void;
  /// A hit elsewhere opens its session, at its entry (SE-C172).
  onHit: (hit: FindHit, raw: SearchHit) => void;
  /// Each hit's own notice: "That session is no longer listed." sits by the hit (SE-C179).
  notices: ReadonlyMap<string, Notice>;
  dismiss: (key: string) => void;
}) {
  const field = useRef<HTMLInputElement>(null);
  const [text, setText] = useState(query);
  const asked = useSettled(text, FIND_DEBOUNCE_MS).trim();
  // Typing replaces the address once it settles.
  useEffect(() => {
    if (asked !== query.trim()) onQuery(asked);
  }, [asked, query, onQuery]);
  const params = findParams(asked, scope, openId);
  const reply = useQuery({
    queryKey: liveKeys.find(asked, scope, scope === 'session' ? openId : null),
    queryFn: ({ signal }) => readFind(params ?? {}, signal),
    enabled: params !== null,
    retry: false,
    refetchOnWindowFocus: false,
    staleTime: 30_000,
  });
  // Opening Find focuses the field, selects what it holds, and searches for it (SE-C163).
  useEffect(() => {
    field.current?.focus();
    field.current?.select();
  }, []);
  // Escape closes Find wherever the focus is (SE-C176).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The field closes on its own Escape and marks it handled; this is every other focus.
      if (e.key === 'Escape' && !e.defaultPrevented) onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const result: FindResult | null =
    params === null
      ? null
      : reply.status === 'success'
        ? { reply: reply.data }
        : reply.status === 'error'
          ? reply.error instanceof Refused
            ? { failed: refusalText(reply.error) }
            : { unreachable: true }
          : null;
  const view = result && findView(result, asked, scope, openId);
  const raw = reply.status === 'success' ? (reply.data.hits ?? []) : [];
  const scopes = findScopes(openId);
  return (
    <section className={styles.find} aria-label="Find">
      <div className={styles.findBar}>
        <Search
          ref={field}
          id="sessions-find"
          size="lg"
          labelText="Find"
          placeholder={findPlaceholder(scope, openId)}
          closeButtonLabelText="Clear what Find searches for"
          value={text}
          onChange={(e) => setText(e.target.value)}
          // Carbon's Search swallows Escape to clear its box; Find closes on it instead (SE-C176).
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              onClose();
            }
          }}
        />
        <div className={styles.findScope}>
          <ContentSwitcher
            size="md"
            selectedIndex={scope === 'session' ? 0 : 1}
            onChange={(e) => {
              const name = (e as { name?: string | number }).name;
              onScope(name === 'session' ? 'session' : 'all');
            }}
          >
            {scopes.map((s) => (
              <Switch key={s.value} name={s.value} text={s.label} disabled={s.disabled} />
            ))}
          </ContentSwitcher>
        </div>
        <p className={styles.findWhat}>Find searches everything this router has recorded, every transcript to its first turn, which the feed’s tail cannot reach.</p>
      </div>
      <div className={styles.findRows} aria-live="polite">
        {params !== null && !view && <p className={styles.findNote}>Searching…</p>}
        {view?.searched && <p className={styles.findNote}>{view.searched}</p>}
        {view?.note && <p className={styles.findNote}>{view.note}</p>}
        {view?.hits.map((h, i) => {
          const body = (
            <>
              <span className={styles.hitTop}>
                <span className={styles.hitKind}>{h.kind}</span>
                <span>{h.role}</span>
                <span className={styles.hitAge}>{h.age}</span>
              </span>
              <span className={styles.snippet}>
                {h.snippet.map((r, j) => (r.mark ? <mark key={j}>{r.text}</mark> : <span key={j}>{r.text}</span>))}
              </span>
              {h.where && <span className={styles.hitWhere}>{h.where}</span>}
            </>
          );
          const said = notices.get(h.key) ?? null;
          const hit = raw[i];
          return (
            <div key={h.key} className={styles.hitWrap}>
              {/* A hit in the session already open is not a link: the feed cannot scroll to a
                  turn it never loaded (SE-C172). */}
              {h.opens && hit ? (
                <button
                  type="button"
                  className={styles.hit}
                  // Its facts with the pauses its layout makes for the eye.
                  aria-label={`${h.kind}, ${h.role}, ${h.age} ago: ${h.snippet.map((r) => r.text).join('')}${h.where ? `. In ${h.where}` : ''}`}
                  onClick={() => onHit(h, hit)}
                >
                  {body}
                </button>
              ) : (
                <div className={styles.hit}>{body}</div>
              )}
              <NoticeLine notice={said} onClose={() => dismiss(h.key)} />
            </div>
          );
        })}
        {view?.more && <p className={styles.findNote}>{view.more}</p>}
      </div>
      <Button kind="ghost" size="md" className={styles.findClose} renderIcon={Close} onClick={onClose}>
        Close Find
      </Button>
    </section>
  );
}
