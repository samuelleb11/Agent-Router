// New for agent-router: Accounts, Projects, Sessions, and Tool output beside Tool calls (assets/index.html,
// `renderAccountSplit`, `renderProjects`, `renderSessions`, `orphanRow`, `renderTools`). Bar rows,
// never a table (OV-C129); a list that scopes the page marks its selected row (OV-C130).
import { Button } from '@carbon/react';
import { Fragment, useState } from 'react';
import { Link } from 'react-router';
import { BarList } from '~/components/BarList';
import type { Snapshot } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { settingsPath } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { count } from '~/model/format';
import { toggle, type Scope } from '~/model/scope';
import { isHosted, settingsPages, type Auth } from '~/model/settings';
import {
  accountRows,
  accountSplit,
  emptyWords,
  familiesOf,
  LIST_PAGE,
  nothingRecorded,
  projectRows,
  scopingCaption,
  sessionRows,
  sessionsCaption,
  toolCallRows,
  toolOutputRows,
  unattributed,
  type Unattributed,
} from './panels';
import { Panel, Swatch } from './parts';
import styles from './Overview.module.scss';

/// The row that counts requests waiting for their transcript, with its own mark, never the error's
/// (OV-C6), and its explanation beside it (OV-C142).
function NotYetAttributed({ row }: { row: Unattributed }) {
  const Glass = CONCEPTS.unattributed.icon;
  return (
    <div className={styles.orphan} data-unattributed="">
      <span className={styles.orphanLine}>
        <Glass aria-hidden="true" style={{ fill: paint(CONCEPTS.unattributed.colour) }} />
        <span className={styles.orphanName}>{row.label}</span>
        <span>{row.meta}</span>
      </span>
      <p className={styles.note}>{row.why}</p>
    </div>
  );
}

/// How to start, where the router says how: Settings → Claude Code on a router that runs on this
/// Mac, the Router page's connection snippet on a hosted one. Nothing until the router has said
/// which it is, so the page never assumes a Mac (OV-C203).
function HowToStart({ auth }: { auth: Auth | null }) {
  if (!auth) return null;
  const local = settingsPages(auth).includes('claude-code');
  return (
    <p className={styles.note}>
      {local ? (
        <>
          <Link to={settingsPath('claude-code')}>Settings → Claude Code</Link> routes every project through it.
        </>
      ) : (
        <>
          <Link to={settingsPath('router')}>Settings → Router</Link> has the line that points a Claude Code at {isHosted(auth) ? 'this deployment' : 'it'}.
        </>
      )}
    </p>
  );
}

function ShowMore({ total, shown, onMore, onFewer }: { total: number; shown: number; onMore: () => void; onFewer: () => void }) {
  const coarse = useCoarse();
  if (total > shown) {
    return (
      <div className={styles.more}>
        <Button kind="tertiary" size={coarse ? 'lg' : 'sm'} onClick={onMore}>
          Show {count(Math.min(LIST_PAGE, total - shown))} more
        </Button>
      </div>
    );
  }
  if (total > LIST_PAGE) {
    return (
      <div className={styles.more}>
        <Button kind="ghost" size={coarse ? 'lg' : 'sm'} onClick={onFewer}>
          Show fewer
        </Button>
      </div>
    );
  }
  return null;
}

export function Lists({ snapshot, scope, now, busy, auth, onScope }: { snapshot: Snapshot; scope: Scope; now: number; busy: boolean; auth: Auth | null; onScope: (next: Scope) => void }) {
  const split = accountSplit(snapshot.summary.by_account);
  const projects = projectRows(snapshot.projects, now);
  const sessions = sessionRows(snapshot.sessions, scope, now);
  // Sessions holds up to 200: a first page and "Show more", never every row at once (OV-C139). The
  // page returns to its first twenty when the scope changes (OV-C33).
  const scopeKey = `${scope.account}|${scope.project}|${scope.session}|${scope.range}`;
  const [paging, setPaging] = useState({ key: scopeKey, shown: LIST_PAGE });
  const shown = paging.key === scopeKey ? paging.shown : LIST_PAGE;
  const orphanProjects = unattributed(snapshot.summary.unattributed, 'project', scope);
  const orphanSessions = unattributed(snapshot.summary.unattributed, 'session', scope);
  const produced = toolOutputRows(snapshot.tools.produced);
  const called = toolCallRows(snapshot.tools.called);
  const start = nothingRecorded(scope);

  return (
    <>
      {split && (
        <Panel concept="account" title="Accounts" caption={scope.account ? scopingCaption('account', split.length, true) : `${split.length} accounts have served traffic · click one to scope the page`} busy={busy}>
          <BarList
            label="Accounts"
            rows={accountRows(split, now).map((a) => ({
              ...a,
              chips: a.active ? <span className={styles.activeTag}>active</span> : undefined,
            }))}
            selected={scope.account}
            onSelect={(key) => onScope(toggle(scope, 'account', key))}
          />
        </Panel>
      )}

      <Panel concept="project" title="Projects" caption={projects.length ? scopingCaption('project', projects.length, !!scope.project) : undefined} busy={busy}>
        {projects.length ? (
          <BarList
            label="Projects"
            rows={projects.map((p, i) => ({
              ...p,
              // One swatch per model family it used, four at most (OV-C135).
              chips: (
                <span className={styles.chips}>
                  {familiesOf(snapshot.projects[i]?.models).map((f) => (
                    <Swatch key={f.family} family={f.family} />
                  ))}
                </span>
              ),
            }))}
            selected={scope.project}
            onSelect={(key) => onScope(toggle(scope, 'project', key))}
          />
        ) : (
          <>
            <p className={styles.empty}>{emptyWords('projects', scope)}</p>
            {start && <HowToStart auth={auth} />}
          </>
        )}
        {orphanProjects && <NotYetAttributed row={orphanProjects} />}
      </Panel>

      <Panel concept="session" title="Sessions" caption={sessions.length ? sessionsCaption(sessions.length, scope) : undefined} busy={busy}>
        {sessions.length ? (
          <>
            <BarList
              label="Sessions"
              rows={sessions.slice(0, shown).map((s) => ({
                ...s,
                chips: (
                  <span className={styles.chips}>
                    {s.chipsOf.branch && (
                      <span className={styles.chip}>
                        <BranchIcon />
                        {s.chipsOf.branch}
                      </span>
                    )}
                    {s.chipsOf.project && (
                      <span className={styles.chip}>
                        <ProjectIcon />
                        {s.chipsOf.project.name}
                      </span>
                    )}
                    {s.chipsOf.client && <span className={styles.chip}>{s.chipsOf.client.name}</span>}
                    {s.chipsOf.families.map((f) => (
                      <Fragment key={f.family}>
                        <Swatch family={f.family} />
                      </Fragment>
                    ))}
                  </span>
                ),
              }))}
              selected={scope.session}
              onSelect={(key) => {
                const row = snapshot.sessions.find((x) => x.id === key);
                onScope(toggle(scope, 'session', key, row?.project ?? null));
              }}
            />
            <ShowMore total={sessions.length} shown={shown} onMore={() => setPaging({ key: scopeKey, shown: shown + LIST_PAGE })} onFewer={() => setPaging({ key: scopeKey, shown: LIST_PAGE })} />
          </>
        ) : (
          <p className={styles.empty}>{emptyWords('sessions', scope)}</p>
        )}
        {orphanSessions && <NotYetAttributed row={orphanSessions} />}
      </Panel>

      <div className={styles.two} data-two="">
        <Panel concept="tools" title="Tool output the model read" caption="est. tokens" busy={busy}>
          {produced.length ? <BarList label="Tool output the model read" rows={produced} /> : <p className={styles.empty}>{emptyWords('toolOutput', scope)}</p>}
        </Panel>
        <Panel concept="tools" title="Tool calls made" caption="by the model" busy={busy}>
          {called.length ? <BarList label="Tool calls made" rows={called} /> : <p className={styles.empty}>{emptyWords('toolCalls', scope)}</p>}
        </Panel>
      </div>
    </>
  );
}

function BranchIcon() {
  const Icon = CONCEPTS.branch.icon;
  return <Icon aria-hidden="true" style={{ fill: paint(CONCEPTS.branch.colour) }} />;
}

function ProjectIcon() {
  const Icon = CONCEPTS.project.icon;
  return <Icon aria-hidden="true" style={{ fill: paint(CONCEPTS.project.colour) }} />;
}
