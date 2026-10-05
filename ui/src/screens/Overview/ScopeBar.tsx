// New for agent-router: the scope bar (assets/index.html, `#scope`, `renderCrumbs`, `menuRows`): one
// breadcrumb of three pickers — account › project › session — the time range and Clear scope.
import { Close } from '@carbon/icons-react';
import { Button, ContentSwitcher, Switch } from '@carbon/react';
import { ScopePicker } from '~/components/ScopePicker';
import type { Snapshot } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { useCoarse } from '~/lib/viewport';
import { count } from '~/model/format';
import { RANGE_ORDER, SESSION_CAP, choose, crumbs, isScoped, parseRange, type Level, type Range, type Scope } from '~/model/scope';
import { ALL, accountOptions, projectOptions, sessionOptions } from './scopeOptions';
import styles from './Overview.module.scss';

export function ScopeBar({ scope, snapshot, now, onScope }: { scope: Scope; snapshot: Snapshot | null; now: number; onScope: (next: Scope) => void }) {
  const coarse = useCoarse();
  const byAccount = snapshot?.summary.by_account ?? [];
  const sessions = snapshot?.sessions ?? [];
  const sessionRow = scope.session ? sessions.find((x) => x.id === scope.session) : undefined;
  const c = crumbs(scope, { accountLabel: byAccount.find((a) => a.id === scope.account)?.label ?? null, branch: sessionRow?.git_branch ?? null });
  const pick = (level: Level) => (key: string) => {
    if (key === ALL) return onScope(choose(scope, level, null));
    const project = level === 'session' ? (sessions.find((x) => x.id === key)?.project ?? null) : undefined;
    onScope(choose(scope, level, key, project));
  };
  const Scoped = CONCEPTS.scope.icon;

  return (
    <div className={styles.scopeBar} role="region" aria-label="Scope" data-scope-bar="">
      <div className={styles.crumbs}>
        <Scoped aria-hidden="true" style={{ fill: paint(CONCEPTS.scope.colour) }} />
        <ScopePicker level="account" crumb={c.account.label} set={c.account.set} value={scope.account ?? ALL} options={accountOptions(byAccount)} onPick={pick('account')} />
        <span className={styles.sep} aria-hidden="true">
          ›
        </span>
        <ScopePicker level="project" crumb={c.project.label} set={c.project.set} value={scope.project ?? ALL} options={snapshot ? projectOptions(snapshot) : [{ key: ALL, name: 'All projects' }]} onPick={pick('project')} />
        <span className={styles.sep} aria-hidden="true">
          ›
        </span>
        <ScopePicker
          level="session"
          crumb={c.session.label}
          set={c.session.set}
          value={scope.session ?? ALL}
          options={sessionOptions(sessions, scope, now)}
          note={sessions.length >= SESSION_CAP ? `the ${count(SESSION_CAP)} most recent sessions` : undefined}
          onPick={pick('session')}
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
