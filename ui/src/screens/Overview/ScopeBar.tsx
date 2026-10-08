// New for agent-router: the scope bar (assets/index.html, `#scope`, `renderCrumbs`, `menuRows`): one
// breadcrumb of three pickers — account › project › session — the time range and Clear scope. A picker
// reads its rows when it is opened, never on the poll (OV-C217); the crumbs name a scoped account and
// session from what the snapshot read for them (OV-C13).
import { Close } from '@carbon/icons-react';
import { Button, ContentSwitcher, Switch } from '@carbon/react';
import { useState } from 'react';
import { ScopePicker, type ScopeOption } from '~/components/ScopePicker';
import { usePickers, type Snapshot } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { refusalText } from '~/lib/errors';
import { useCoarse } from '~/lib/viewport';
import { count } from '~/model/format';
import { RANGE_ORDER, SESSION_CAP, choose, crumbs, isScoped, parseRange, type Level, type Range, type Scope } from '~/model/scope';
import { ALL, accountOptions, projectOptions, sessionOptions } from './scopeOptions';
import styles from './Overview.module.scss';

/// What a picker says above its rows while they are read, or when they could not be.
function reading(read: { isFetching: boolean; isError: boolean; error: unknown; data?: unknown }, plural: string): string | undefined {
  if (read.isError) return `could not read the ${plural}: ${refusalText(read.error)}`;
  if (read.isFetching && read.data === undefined) return `reading the ${plural}…`;
  return undefined;
}

export function ScopeBar({ scope, snapshot, now, onScope }: { scope: Scope; snapshot: Snapshot | null; now: number; onScope: (next: Scope) => void }) {
  const coarse = useCoarse();
  const [opened, setOpened] = useState<Level | null>(null);
  const lists = usePickers(scope, opened);
  const byAccount = lists.account.data?.by_account ?? [];
  const projects = lists.project.data ?? [];
  const sessions = lists.session.data ?? [];
  const accountLabel = (snapshot?.summary.by_account ?? byAccount).find((a) => a.id === scope.account)?.label ?? null;
  const branch = (snapshot?.session ?? sessions.find((x) => x.id === scope.session))?.git_branch ?? null;
  const c = crumbs(scope, { accountLabel, branch });
  const pick = (level: Level) => (key: string) => {
    if (key === ALL) return onScope(choose(scope, level, null));
    const project = level === 'session' ? (sessions.find((x) => x.id === key)?.project ?? null) : undefined;
    onScope(choose(scope, level, key, project));
  };
  const toggled = (level: Level) => (open: boolean) => setOpened((was) => (open ? level : was === level ? null : was));
  const Scoped = CONCEPTS.scope.icon;
  const accounts: ScopeOption[] = lists.account.data ? accountOptions(byAccount) : [{ key: ALL, name: 'All accounts' }];

  return (
    <div className={styles.scopeBar} role="region" aria-label="Scope" data-scope-bar="">
      <div className={styles.crumbs}>
        <Scoped aria-hidden="true" style={{ fill: paint(CONCEPTS.scope.colour) }} />
        <ScopePicker
          level="account"
          crumb={c.account.label}
          set={c.account.set}
          value={scope.account ?? ALL}
          options={accounts}
          note={reading(lists.account, 'accounts')}
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
          note={reading(lists.project, 'projects')}
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
          options={lists.session.data ? sessionOptions(sessions, scope, now) : [{ key: ALL, name: 'All sessions' }]}
          note={reading(lists.session, 'sessions') ?? (sessions.length >= SESSION_CAP ? `the ${count(SESSION_CAP)} most recent sessions` : undefined)}
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
        <Button kind="ghost" size={coarse ? 'lg' : 'sm'} renderIcon={Close} onClick={() => onScope({ ...scope, account: null, project: null, session: null })}>
          Clear scope
        </Button>
      )}
    </div>
  );
}
