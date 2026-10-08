// New for agent-router: Requests (assets/index.html, `#requests`, `renderRequests`, `renderDetail`): the
// 300 newest in scope as cards that open in place, the filters over what is loaded, and Show more.
import { Button, ContentSwitcher, SelectableTag, Switch } from '@carbon/react';
import { RequestRow } from '~/components/RequestRow';
import type { RequestRecord } from '~/data/overview';
import { useCoarse } from '~/lib/viewport';
import { count, family, shortModel } from '~/model/format';
import type { RequestFilters, RequestKind, Scope } from '~/model/scope';
import { badges, detail, figures, filtering, moreOrFewer, REQUEST_PAGE, requestsCaption, requestTime, startedAt } from './requestList';
import { Panel, Swatch } from './parts';
import styles from './Overview.module.scss';

/// The id the Errors tile's button brings into view (OV-C107).
export const REQUESTS_ID = 'requests';

export function Requests({
  requests: list,
  p95,
  scope,
  filters,
  open,
  now,
  busy,
  shown,
  onShown,
  onFilters,
  onOpen,
}: {
  /// The 300 newest in scope, read for the Requests tab.
  requests: readonly RequestRecord[];
  /// The top's time-to-first-byte p95, which marks a request slow.
  p95: number | null;
  scope: Scope;
  /// What the address asks for.
  filters: RequestFilters;
  /// The rows open, by id, from the address (OV-C157).
  open: readonly string[];
  now: number;
  busy: boolean;
  shown: number;
  onShown: (n: number) => void;
  onFilters: (next: RequestFilters) => void;
  onOpen: (ids: readonly string[]) => void;
}) {
  const coarse = useCoarse();
  const f = filtering(list, filters, p95);
  const rows = f.rows;
  const page = rows.slice(0, shown);
  const max = rows.reduce((m, r) => Math.max(m, r.prompt_tokens ?? 0), 0);
  const step = moreOrFewer(rows.length, shown);
  // A filter change returns the list to its first twenty rows (OV-C154).
  const filter = (next: RequestFilters) => {
    onShown(REQUEST_PAGE);
    onFilters(next);
  };
  const toggleOpen = (id: string) => onOpen(open.includes(id) ? open.filter((x) => x !== id) : [...open, id]);

  return (
    <Panel id={REQUESTS_ID} concept="requests" title="Requests" caption={list.length ? requestsCaption(list.length, rows.length, shown, scope) : undefined} busy={busy}>
      {list.length > 0 && (
        <div className={styles.filters} data-request-filters="">
          <div className={styles.filterRow}>
            {/* The kinds: All always, the others when a loaded row is of them or they are on, each
                with its count (OV-C148, OV-C153). */}
            <ContentSwitcher
              className={styles.kindSwitch}
              aria-label="Which requests"
              size={coarse ? 'lg' : 'sm'}
              selectedIndex={Math.max(0, f.kinds.findIndex((k) => k.kind === f.effective.kind))}
              onChange={({ name }: { name?: string | number }) => {
                const kind = (f.kinds.find((k) => k.kind === name)?.kind ?? 'all') as RequestKind;
                if (kind !== f.effective.kind) filter({ ...f.effective, kind });
              }}
            >
              {f.kinds.map((k) => (
                <Switch key={k.kind} name={k.kind} text={`${k.label} ${count(k.count)}`} />
              ))}
            </ContentSwitcher>
            {f.slowRule && <span className={styles.note}>Slow: {f.slowRule}</span>}
          </div>
          {f.models.length > 0 && (
            <div className={styles.filterRow} role="group" aria-label="Models">
              {f.models.map((m) => (
                <SelectableTag
                  key={m.model}
                  size={coarse ? 'lg' : 'md'}
                  text={`${m.short} ${count(m.count)}`}
                  renderIcon={() => <Swatch family={m.family} />}
                  selected={f.effective.model === m.model}
                  onChange={(on: boolean) => filter({ ...f.effective, model: on ? m.model : null })}
                />
              ))}
              {f.effective.model && <span className={styles.note}>model {f.effective.model}</span>}
            </div>
          )}
          {f.stops.length > 0 && (
            <div className={styles.filterRow} role="group" aria-label="Stop reasons">
              {f.stops.map((s) => (
                <SelectableTag
                  key={s.stop}
                  size={coarse ? 'lg' : 'md'}
                  text={`${s.stop} ${count(s.count)}`}
                  selected={f.effective.stop === s.stop}
                  onChange={(on: boolean) => filter({ ...f.effective, stop: on ? s.stop : null })}
                />
              ))}
            </div>
          )}
        </div>
      )}
      {!list.length ? (
        <p className={styles.empty}>No requests for this scope.</p>
      ) : !rows.length ? (
        // Which empty, and the way out (OV-C155).
        <div className={styles.more}>
          <p className={styles.empty}>No request matches these filters.</p>
          <Button kind="ghost" size={coarse ? 'lg' : 'sm'} onClick={() => filter({ kind: 'all', model: null, stop: null })}>
            Clear the filters
          </Button>
        </div>
      ) : (
        <ul className={styles.rows} aria-label="Requests, newest first">
          {page.map((r: RequestRecord) => {
            const b = badges(r, p95);
            const id = String(r.id);
            return (
              <li key={id}>
                <RequestRow
                  time={requestTime(r.started_at, now)}
                  at={startedAt(r)}
                  model={r.model}
                  modelShort={shortModel(r.model)}
                  family={family(r.model)}
                  subagent={b.subagent}
                  slow={b.slow}
                  status={r.status}
                  error={b.error}
                  prompt={r.prompt_tokens}
                  max={max}
                  figures={figures(r)}
                  open={open.includes(id)}
                  onToggle={() => toggleOpen(id)}
                  detail={() => detail(r, scope)}
                />
              </li>
            );
          })}
        </ul>
      )}
      {step && (
        <div className={styles.more}>
          {'more' in step ? (
            <Button kind="tertiary" size={coarse ? 'lg' : 'sm'} onClick={() => onShown(shown + REQUEST_PAGE)}>
              Show {count(step.more)} more
            </Button>
          ) : (
            <Button kind="ghost" size={coarse ? 'lg' : 'sm'} onClick={() => onShown(REQUEST_PAGE)}>
              Show fewer
            </Button>
          )}
        </div>
      )}
    </Panel>
  );
}
