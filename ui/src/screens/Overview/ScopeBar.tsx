// New for agent-router: the scope bar (assets/index.html, `#scope`, `renderCrumbs`, `menuRows`): the
// provider, then one breadcrumb of three pickers — account › project › session — the time range and
// Clear scope. The provider is a switcher, every choice visible at rest, as the range is (OV-C227). A
// picker reads its rows when it is opened, never on the poll (OV-C217); the crumbs name a scoped
// account and session from what the snapshot read for them (OV-C13).
import { Close } from '@carbon/icons-react';
import { Button, ContentSwitcher, Switch } from '@carbon/react';
import { useState } from 'react';
import { ProviderMark } from '~/components/ProviderMark';
import { ScopePicker, type ScopeOption } from '~/components/ScopePicker';
import { usePickers, type Snapshot } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { refusalText } from '~/lib/errors';
import { useCoarse } from '~/lib/viewport';
import { count } from '~/model/format';
import { RANGE_ORDER, SESSION_CAP, choose, chooseProvider, clearScope, crumbs, isScoped, parseProvider, parseRange, type Level, type Range, type Scope } from '~/model/scope';
import type { Account } from '~/model/settings';
import { ALL, accountOptions, accountProvider, pickerNote, projectOptions, providerChoices, sessionOptions } from './scopeOptions';
import styles from './Overview.module.scss';

/// What a picker says above its rows while they are read, or when they could not be.
function reading(read: { isFetching: boolean; isError: boolean; error: unknown; data?: unknown }, plural: string): string | undefined {
  if (read.isError) return `could not read the ${plural}: ${refusalText(read.error)}`;
  if (read.isFetching && read.data === undefined) return `reading the ${plural}…`;
  return undefined;
}

export function ScopeBar({
  scope,
  snapshot,
  accounts,
  now,
  onScope,
}: {
  scope: Scope;
  snapshot: Snapshot | null;
  /// The router's accounts, which say whose a scoped account is; null until read.
  accounts: readonly Account[] | null;
  now: number;
  onScope: (next: Scope) => void;
}) {
  const coarse = useCoarse();
  const [opened, setOpened] = useState<Level | null>(null);
  const lists = usePickers(scope, opened);
  const byAccount = lists.account.data?.by_account ?? [];
  const projects = lists.project.data ?? [];
  const sessions = lists.session.data ?? [];
  const accountLabel = (snapshot?.summary.by_account ?? byAccount).find((a) => a.id === scope.account)?.label ?? null;
  // Each provider with an account, as Quota headroom has them: the summary's `providers`, which no
  // scope narrows (OV-C224, OV-C227).
  const offered = providerChoices((snapshot?.summary.providers ?? []).map((b) => b.provider), scope.provider);
  const pickProvider = (name: string | number | undefined) => {
    const provider = parseProvider(name);
    if (provider === scope.provider) return;
    // An account picked from another provider is let go; the project and the session stay (OV-C227).
    onScope(chooseProvider(scope, provider, accountProvider(scope.account, accounts, snapshot?.summary.by_account, byAccount)));
  };
  const branch = (snapshot?.session ?? sessions.find((x) => x.id === scope.session))?.git_branch ?? null;
  const c = crumbs(scope, { accountLabel, branch });
  const pick = (level: Level) => (key: string) => {
    if (key === ALL) return onScope(choose(scope, level, null));
    const project = level === 'session' ? (sessions.find((x) => x.id === key)?.project ?? null) : undefined;
    onScope(choose(scope, level, key, project));
  };
  const toggled = (level: Level) => (open: boolean) => setOpened((was) => (open ? level : was === level ? null : was));
  const Scoped = CONCEPTS.scope.icon;
  // An account's provider is marked before its name, and named on its second line, so the mark is not
  // heard twice (OV-C231); a session's providers are marked alone, each named by its mark (SE-C242).
  const accountChoices: ScopeOption[] = lists.account.data
    ? accountOptions(byAccount).map(({ provider, ...o }) => (provider ? { ...o, mark: <ProviderMark provider={provider} decorative /> } : o))
    : [{ key: ALL, name: 'All accounts' }];
  const sessionChoices: ScopeOption[] = lists.session.data
    ? sessionOptions(sessions, scope, now).map(({ providers, ...o }) =>
        providers ? { ...o, mark: providers.map((p) => <ProviderMark key={p} provider={p} alone />) } : o,
      )
    : [{ key: ALL, name: 'All sessions' }];

  return (
    <div className={styles.scopeBar} role="region" aria-label="Scope" data-scope-bar="">
      <div className={styles.crumbs}>
        <Scoped aria-hidden="true" style={{ fill: paint(CONCEPTS.scope.colour) }} />
        {offered && (
          <>
            {/* Every provider, then each with an account by its mark and name, all visible at rest,
                beside the account picker it narrows (OV-C227). Low contrast, so the chosen one keeps the
                page's surface: Carbon's default inverts it, and a mark is drawn for the surface it sits
                on — Anthropic's in the text colour, Kimi's file for the theme (SH-C56) — so on an
                inverted switch Anthropic's would vanish. */}
            <ContentSwitcher
              className={styles.providers}
              aria-label="Provider"
              lowContrast
              size={coarse ? 'lg' : 'sm'}
              selectedIndex={Math.max(0, offered.indexOf(scope.provider))}
              onChange={({ name }: { name?: string | number }) => pickProvider(name)}
            >
              {offered.map((p) =>
                p ? (
                  <Switch key={p} name={p} data-provider={p}>
                    <ProviderMark provider={p} />
                  </Switch>
                ) : (
                  <Switch key={ALL} name={ALL} text="All providers" />
                ),
              )}
            </ContentSwitcher>
            <span className={`${styles.sep} ${styles.providerSep}`} aria-hidden="true">
              ›
            </span>
          </>
        )}
        <ScopePicker
          level="account"
          crumb={c.account.label}
          set={c.account.set}
          value={scope.account ?? ALL}
          options={accountChoices}
          note={reading(lists.account, 'accounts') ?? pickerNote(byAccount)}
          onPick={pick('account')}
          onOpenChange={toggled('account')}
        />
        <span className={styles.sep} aria-hidden="true">
          ›
        </span>
        <ScopePicker
          level="project"
          crumb={c.project.label}
          set={c.project.set}
          value={scope.project ?? ALL}
          options={lists.project.data ? projectOptions(projects, snapshot?.summary.totals.requests ?? null) : [{ key: ALL, name: 'All projects' }]}
          note={reading(lists.project, 'projects') ?? pickerNote(projects)}
          onPick={pick('project')}
          onOpenChange={toggled('project')}
        />
        <span className={styles.sep} aria-hidden="true">
          ›
        </span>
        <ScopePicker
          level="session"
          crumb={c.session.label}
          set={c.session.set}
          value={scope.session ?? ALL}
          options={sessionChoices}
          note={reading(lists.session, 'sessions') ?? ([sessions.length >= SESSION_CAP ? `the ${count(SESSION_CAP)} most recent sessions` : null, pickerNote(sessions)].filter(Boolean).join(' · ') || undefined)}
          onPick={pick('session')}
          onOpenChange={toggled('session')}
        />
      </div>
      <ContentSwitcher
        className={styles.range}
        aria-label="Time range"
        size={coarse ? 'lg' : 'sm'}
        selectedIndex={Math.max(0, RANGE_ORDER.indexOf(scope.range))}
        onChange={({ name }: { name?: string | number }) => {
          const range: Range = parseRange(name);
          if (range !== scope.range) onScope({ ...scope, range });
        }}
      >
        {RANGE_ORDER.map((r) => (
          <Switch key={r} name={r} text={r} />
        ))}
      </ContentSwitcher>
      {isScoped(scope) && (
        <Button kind="ghost" size={coarse ? 'lg' : 'sm'} renderIcon={Close} onClick={() => onScope(clearScope(scope))}>
          Clear scope
        </Button>
      )}
    </div>
  );
}
