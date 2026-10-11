// New for agent-router: the Projects & sessions tab — Accounts, Projects, Sessions — and the Tools tab — Tool
// output beside Tool calls (assets/index.html, `renderAccountSplit`, `renderProjects`, `renderSessions`,
// `orphanRow`, `renderTools`). Bar rows, never a table (OV-C129); a list that scopes the page marks its
// selected row (OV-C130).
import { Button } from '@carbon/react';
import { Fragment, useState } from 'react';
import { Link } from 'react-router';
import { BarList } from '~/components/BarList';
import { ProviderMark } from '~/components/ProviderMark';
import type { TabPart, Tools as ToolsRead } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { settingsPath } from '~/lib/routes';
import { useCoarse } from '~/lib/viewport';
import { costNote, count, type CostFigure } from '~/model/format';
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
  providerWords,
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

/// How to start, each client by its name and its own way, side by side (OV-C230): for Claude Code,
/// Settings → Claude Code on a router that runs on this Mac, the Router page's connection snippet on
/// a hosted one; for Kimi Code, the two commands that point it here. Nothing until the router has
/// said which it is, so the page never assumes a Mac (OV-C203).
function HowToStart({ auth }: { auth: Auth | null }) {
  if (!auth) return null;
  const local = settingsPages(auth).includes('claude-code');
  return (
    <ul className={styles.howToStart} aria-label="How to start" data-how-to-start="">
      <li className={styles.note} data-client="claude-code">
        <strong>Claude Code</strong>{' '}
        {local ? (
          <>
            <Link to={settingsPath('claude-code')}>Settings → Claude Code</Link> routes every project through it.
          </>
        ) : (
          <>
            <Link to={settingsPath('router')}>Settings → Router</Link> has the line that points a Claude Code at {isHosted(auth) ? 'this deployment' : 'it'}.
          </>
        )}
      </li>
      <li className={styles.note} data-client="kimi-code">
        <strong>Kimi Code</strong> <code className={styles.mono}>agent-router install</code> writes the router into Kimi Code’s config.toml, and{' '}
        <code className={styles.mono}>agent-router kimi</code> starts a hosted session from a terminal.
      </li>
      <li className={styles.note} data-client="codex">
        <strong>Codex</strong>{' '}
        {local ? <><code className={styles.mono}>agent-router install</code> connects Codex through the router.</> : <>The machine installer connects Codex to this router.</>}{' '}
        Start it in its terminal or VS Code, then follow its sessions here.
      </li>
    </ul>
  );
}

/// What the marks on a list's costs mean, once, under the list: its rows are buttons, whose tooltip
/// is only a title (OV-C229, OV-C232, SH-C30).
function CostNote({ rows }: { rows: readonly { readonly cost: CostFigure }[] }) {
  const note = costNote(rows.map((r) => r.cost));
  return note ? (
    <p className={styles.note} data-cost-note="">
      {note}
    </p>
  ) : null;
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

/// The Projects & sessions tab, from what was read for it and the top's count of requests no session
/// holds, which ends the Projects and Sessions lists (OV-C141).
export function Lists({
  part,
  unattributed: orphans,
  scope,
  now,
  busy,
  auth,
  onScope,
}: {
  part: Extract<TabPart, { tab: 'projects' }>;
  unattributed: number | null;
  scope: Scope;
  now: number;
  busy: boolean;
  auth: Auth | null;
  onScope: (next: Scope) => void;
}) {
  const split = accountSplit(part.byAccount);
  const projects = projectRows(part.projects, now);
  const sessions = sessionRows(part.sessions, scope, now);
  // Sessions holds up to 200: a first page and "Show more", never every row at once (OV-C139). The
  // page returns to its first twenty when the scope changes (OV-C33).
  const scopeKey = `${scope.provider}|${scope.account}|${scope.project}|${scope.session}|${scope.range}`;
  const [paging, setPaging] = useState({ key: scopeKey, shown: LIST_PAGE });
  const shown = paging.key === scopeKey ? paging.shown : LIST_PAGE;
  const orphanProjects = unattributed(orphans, 'project', scope);
  const orphanSessions = unattributed(orphans, 'session', scope);
  const start = nothingRecorded(scope);

  return (
    <>
      {split && (
        <Panel concept="account" title="Accounts" caption={scope.account ? scopingCaption('account', split.length, true) : `${split.length} accounts have served traffic · click one to scope the page`} busy={busy}>
          <BarList
            label="Accounts"
            rows={accountRows(split, now).map((a) => ({
              ...a,
              // Its provider's mark and name, and while it signs that provider's new requests, "active
              // for" it: an Anthropic account and a Kimi Code account can both be active (OV-C231).
              chips: <ProviderMark provider={a.provider} words={providerWords(a.provider, a.active)} className={a.active ? styles.activeTag : styles.chip} />,
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
                  {familiesOf(part.projects[i]?.models).map((f) => (
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
        <CostNote rows={projects} />
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
                    {/* The providers of its requests, each its mark alone, beside its client (SE-C242). */}
                    <span className={styles.chip} data-providers="">
                      {s.providers.map((p) => (
                        <ProviderMark key={p} provider={p} alone />
                      ))}
                      {s.chipsOf.client?.name}
                    </span>
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
                const row = part.sessions.find((x) => x.id === key);
                onScope(toggle(scope, 'session', key, row?.project ?? null));
              }}
            />
            <ShowMore total={sessions.length} shown={shown} onMore={() => setPaging({ key: scopeKey, shown: shown + LIST_PAGE })} onFewer={() => setPaging({ key: scopeKey, shown: LIST_PAGE })} />
            <CostNote rows={sessions.slice(0, shown)} />
          </>
        ) : (
          <p className={styles.empty}>{emptyWords('sessions', scope)}</p>
        )}
        {orphanSessions && <NotYetAttributed row={orphanSessions} />}
      </Panel>
    </>
  );
}

/// The Tools tab: Tool output the model read beside Tool calls made.
export function Tools({ tools, scope, busy }: { tools: ToolsRead; scope: Scope; busy: boolean }) {
  const produced = toolOutputRows(tools.produced);
  const called = toolCallRows(tools.called);
  return (
    <div className={styles.two} data-two="">
      <Panel concept="tools" title="Tool output the model read" caption="est. tokens" busy={busy}>
        {produced.length ? <BarList label="Tool output the model read" rows={produced} /> : <p className={styles.empty}>{emptyWords('toolOutput', scope)}</p>}
      </Panel>
      <Panel concept="tools" title="Tool calls made" caption="by the model" busy={busy}>
        {called.length ? <BarList label="Tool calls made" rows={called} /> : <p className={styles.empty}>{emptyWords('toolCalls', scope)}</p>}
      </Panel>
    </div>
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
