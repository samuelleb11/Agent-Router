// New for agent-router: the live view's facts strip and its switchers (assets/live.html, `facts`,
// `switcher`), read for one session and aimed only at it (SE-C136): the screen around it is keyed on
// that session's id, and its reads are keyed on it too, so a slow read for one session can never
// dress another.
import { useId, useRef, useState, type FormEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Modal, Select, SelectItem, TextInput, Toggletip, ToggletipButton, ToggletipContent } from '@carbon/react';
import { Information } from '@carbon/icons-react';
import { FormModal } from '~/components/FormModal';
import { liveKeys, postLive, readOptions, readProject, readStats, type Stats } from '~/data/live';
import { refusalText } from '~/lib/errors';
import { liveProjectKey, notice, OTHER_MODEL, switcherOptions, switchNotice, type LiveOptions, type SessionRow } from '~/model/session';
import { failed, projectFacts, sessionFacts } from './logic';
import { NoticeLine } from './Notice';
import { useNotice } from './useNotice';
import styles from './Sessions.module.scss';

type Kind = 'model' | 'effort' | 'mode';

const KINDS: readonly { readonly kind: Kind; readonly label: string }[] = [
  { kind: 'model', label: 'Model' },
  { kind: 'effort', label: 'Effort' },
  { kind: 'mode', label: 'Mode' },
];

const BYPASS = 'bypassPermissions';

/// The strip: what the session is and what it is costing, the project's own total apart, and for a
/// session under the wrapper the three switchers (SE-C131, SE-C132, SE-C137). Totals are read once
/// each time the session opens, never pushed with every event (SE-C133).
export function Facts({ session, row }: { session: string; row: SessionRow | null }) {
  const once = { staleTime: 0, refetchOnWindowFocus: false, refetchOnReconnect: false, retry: false } as const;
  const stats = useQuery({ queryKey: liveKeys.stats(session), queryFn: ({ signal }) => readStats(session, signal), ...once });
  // The project's total is asked by its key: the repository, so every worktree and clone of it
  // counts, else the directory (SE-C232).
  const key = row ? liveProjectKey(row) : null;
  const project = useQuery({ queryKey: liveKeys.project(key ?? ''), queryFn: ({ signal }) => readProject(key ?? '', signal), enabled: !!key, ...once });
  const options = useQuery({ queryKey: liveKeys.options, queryFn: ({ signal }) => readOptions(signal), staleTime: Infinity, retry: false });
  const [said, say] = useNotice();
  const total = projectFacts(project.data);
  return (
    <section className={styles.facts} aria-label="This session’s totals and settings">
      <p className={styles.factsLine}>
        {stats.isPending ? (
          <span className={styles.muted}>reading totals…</span>
        ) : stats.isError ? (
          // Never "unknown", zeros or "undefined turns" read off a refusal (SE-C135).
          <span className={styles.muted}>Could not read this session’s totals: {refusalText(stats.error)}</span>
        ) : (
          sessionFacts(stats.data).map((f, i) => (
            <span key={i} className={styles.fact}>
              {f}
            </span>
          ))
        )}
        {total && <span className={styles.projectTotal}>{total}</span>}
      </p>
      {row?.wrapped && !stats.isPending ? (
        <Switchers session={session} stats={stats.data ?? null} options={options.data ?? null} say={say} />
      ) : row && !row.wrapped ? (
        <p className={styles.muted}>View only: start this session after installing the wrapper to switch its model, effort or mode from here.</p>
      ) : null}
      <NoticeLine notice={said} onClose={() => say(null)} />
    </section>
  );
}

/// Model, effort and mode as inline Selects, a phone's native picker each. What they offer is the
/// router's (SE-C138); each leads with what is in force, or says it is unknown (SE-C139, SE-C140).
function Switchers({ session, stats, options, say }: { session: string; stats: Stats | null; options: LiveOptions | null; say: (n: ReturnType<typeof notice>) => void }) {
  const id = useId();
  // What is in force: as read, then as switched here (SE-C145).
  const [set, setSet] = useState<Partial<Record<Kind, string>>>({});
  const [busy, setBusy] = useState<Partial<Record<Kind, boolean>>>({});
  const [asking, setAsking] = useState<'other' | 'bypass' | null>(null);
  const [typed, setTyped] = useState('');
  const flying = useRef<Partial<Record<Kind, boolean>>>({});
  const read = (kind: Kind): string => (stats?.[kind] ? String(stats[kind]) : '').replace(/^claude-/, '');
  const current = (kind: Kind): string => (set[kind] ?? read(kind)).replace(/^claude-/, '');
  /// What a switcher offers: the router's list led by what is in force, and the value the session
  /// was read with kept among them, so switching back to it is a choice like any other (SE-C145).
  const offered = (kind: Kind) => {
    const list = switcherOptions(kind, current(kind), options ?? {});
    const was = read(kind);
    if (!was || list.some((o) => o.value === was)) return list;
    return [list[0], { value: was, label: was, current: false }, ...list.slice(1)].filter((o): o is (typeof list)[number] => o !== undefined);
  };

  /// Send a switch. Disabled while it flies; the outcome is the router's (SE-C143, SE-C144).
  const send = async (kind: Kind, value: string) => {
    if (flying.current[kind] || !value || value === current(kind)) return;
    flying.current[kind] = true;
    setBusy((b) => ({ ...b, [kind]: true }));
    const r = await postLive<Record<string, unknown>>(`/api/live/${kind}`, { session, [kind]: value });
    flying.current[kind] = false;
    setBusy((b) => ({ ...b, [kind]: false }));
    if (r.ok) {
      const took = typeof r.body[kind] === 'string' ? String(r.body[kind]) : value;
      setSet((s) => ({ ...s, [kind]: took }));
      say(switchNotice(kind, value, 200, r.body));
    } else {
      // The switcher is put back: it shows what is in force, which did not change.
      say(notice(failed(kind, r)));
    }
  };

  const chose = (kind: Kind, value: string) => {
    if (kind === 'model' && value === OTHER_MODEL) {
      setTyped('');
      setAsking('other');
      return;
    }
    if (value === current(kind)) return;
    // One choice would otherwise stop the session asking about anything, on a machine the reader
    // may not be sitting at: asked first, in a modal (SE-C142).
    if (kind === 'mode' && value === BYPASS) {
      setAsking('bypass');
      return;
    }
    void send(kind, value);
  };

  const otherSubmit = (e: FormEvent) => {
    e.preventDefault();
    const value = typed.trim();
    setAsking(null);
    // An empty answer changes nothing (SE-C141).
    if (value) void send('model', value);
  };

  return (
    <div className={styles.switchers}>
      {KINDS.map(({ kind, label }) => (
        <Select
          key={kind}
          id={`${id}-${kind}`}
          inline
          size="sm"
          className={styles.switcher}
          labelText={label}
          // The option in force reads "model: opus", so the label is said, not shown twice.
          hideLabel
          value={current(kind)}
          disabled={!!busy[kind]}
          onChange={(e) => chose(kind, e.target.value)}
        >
          {offered(kind).map((o) => (
            <SelectItem key={`${o.value}|${o.label}`} value={o.value} text={o.label} />
          ))}
        </Select>
      ))}
      <Toggletip align="bottom-end" className={styles.help}>
        <ToggletipButton label="What these switch">
          <Information />
        </ToggletipButton>
        <ToggletipContent>
          <p>
            The model and the effort this session uses, and its mode: what it stops to ask you about. Each change takes effect on its next turn. Other… names
            a model the list does not hold.
          </p>
        </ToggletipContent>
      </Toggletip>
      <FormModal
        open={asking === 'other'}
        title="Use another model"
        modalLabel="Model"
        submitText="Use this model"
        submitDisabled={typed.trim() === ''}
        onSubmit={otherSubmit}
        onClose={() => setAsking(null)}
        focusId={`${id}-other`}
      >
        <TextInput
          id={`${id}-other`}
          labelText="Model name or alias"
          placeholder="opus, sonnet[1m], a full id…"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
        />
      </FormModal>
      <Modal
        open={asking === 'bypass'}
        danger
        size="sm"
        modalHeading="Bypass permissions?"
        modalLabel="Mode"
        primaryButtonText="Bypass permissions"
        secondaryButtonText="Keep the current mode"
        onRequestClose={() => setAsking(null)}
        onRequestSubmit={() => {
          setAsking(null);
          void send('mode', BYPASS);
        }}
      >
        <p>This session will stop asking before it runs commands or edits files.</p>
      </Modal>
    </div>
  );
}
