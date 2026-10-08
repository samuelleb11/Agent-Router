// @vitest-environment node
// New for agent-router: the share bars' and lists' contracts (docs/web-console-contracts.md, "Read and
// context", "Models and threads" and "Lists"), on what each panel is handed, with no DOM.
import { describe, expect, it } from 'vitest';
import { snapshotPaths } from '~/data/overview';
import { projectKey } from '~/data/projects';
import { routes } from '~/lib/routes';
import { UNSCOPED, type Scope } from '~/model/scope';
import {
  accountRows,
  accountSplit,
  CONTEXT,
  CONTEXT_NOTE,
  contextParts,
  emptyWords,
  modelRows,
  projectKeyOf,
  projectPlaces,
  projectRows,
  readPanel,
  sessionRows,
  sessionsCaption,
  threadParts,
  toolCallRows,
  toolOutputRows,
  unattributed,
} from './panels';
import { NOW, project, session, totals, S1, MIN, HOUR } from './fixtures';

const scoped = (over: Partial<Scope>): Scope => ({ ...UNSCOPED, ...over });

describe('what the model read', () => {
  it('OV-C118: one bar of three parts in a fixed order on the token ramp, the write row split into its tiers', () => {
    const p = readPanel(totals(), UNSCOPED);
    expect(p.parts.map((x) => [x.key, x.label, x.colour])).toEqual([
      ['read', 'from cache', '--ar-tok-3'],
      ['write', 'written to cache', '--ar-tok'],
      ['input', 'paid in full', '--ar-tok-2'],
    ]);
    expect(p.parts[1]!.detail).toBe('5-minute 1.10M · 1-hour 300k');
    expect(p.total).toBe(12_400_000);
  });

  it("OV-C119: paid in full is the router's own uncached input, never a difference worked out on the page (decision J: there is no older router)", () => {
    // Read and written leave 600k of the prompt; the router counted 250k paid in full, and that is what is drawn.
    expect(readPanel(totals({ input_tokens: 250_000 }), UNSCOPED).parts[2]!.value).toBe(250_000);
    // Not sent is not recorded: no figure is made up in its place.
    expect(readPanel(totals({ input_tokens: null }), UNSCOPED).parts[2]).toMatchObject({ value: null, figure: '—' });
  });

  it('OV-C120: the note gives the hit rate as its formula, or says nothing was read', () => {
    expect(readPanel(totals(), UNSCOPED).note).toBe('Hit rate = cache reads ÷ everything read = 84%. Cached tokens are far cheaper against the quota than tokens paid in full.');
    expect(readPanel(totals({ prompt_tokens: 0 }), UNSCOPED).note).toBe('Nothing read yet.');
  });
});

describe('the context window', () => {
  const rows = [
    { component: 'history', est_tokens: 6_000_000, chars: 24_000_000, est_cost_once: 90, requests: 400 },
    { component: 'tools', est_tokens: 1_600_000, chars: 6_400_000, est_cost_once: 24, requests: 400 },
    { component: 'tool_results', est_tokens: 4_000_000, chars: 16_000_000, est_cost_once: 60, requests: 200 },
  ];

  it('OV-C121: four parts in a fixed order with fixed labels, colours and icons, whatever order the router sends them in', () => {
    expect(contextParts(rows).map((p) => [p.key, p.label, p.colour, p.concept])).toEqual([
      ['tools', 'tools (definitions)', '--ar-ctx-tools', 'tools'],
      ['system', 'system prompt', '--ar-ctx-system', 'systemPrompt'],
      ['history', 'conversation', '--ar-ctx-history', 'conversation'],
      ['tool_results', 'tool results', '--ar-ctx-results', 'toolResults'],
    ]);
    expect(CONTEXT).toHaveLength(4);
  });

  it('OV-C122: the split is labelled an estimate, with the note that totals are exact and the split apportions by character share', () => {
    expect(CONTEXT_NOTE).toMatch(/^Totals are exact\. The split apportions the API's exact prompt-token count by character share/);
    expect(CONTEXT_NOTE).toMatch(/no local tokenizer for current Claude models/);
    expect(CONTEXT_NOTE).toMatch(/ratios as reliable and absolutes as approximate/);
  });

  it('OV-C123: each part gives "≈ per request" over the requests that carried it, and none where the router did not say', () => {
    const parts = contextParts([...rows, { component: 'system', est_tokens: 800_000, chars: 3_200_000, est_cost_once: 12, requests: null }]);
    expect(parts.find((p) => p.key === 'tool_results')!.detail).toMatch(/^≈20k per request/);
    expect(parts.find((p) => p.key === 'history')!.detail).toMatch(/^≈15k per request/);
    expect(parts.find((p) => p.key === 'system')!.detail).not.toMatch(/per request/);
  });

  it('OV-C124: a part’s cost reads as the floor it is, "≥$X if paid once as input", with its characters, as text', () => {
    expect(contextParts(rows).find((p) => p.key === 'history')!.detail).toBe('≈15k per request · ≥$90.00 if paid once as input · 24.00M characters');
    // A part the router sent nothing for says nothing, rather than "0".
    expect(contextParts(rows).find((p) => p.key === 'system')).toMatchObject({ value: null, figure: '—', detail: undefined });
  });
});

describe('models and threads', () => {
  it('OV-C126: one row per model in the router’s order, its short name with the full id, the family colour, and its figures, errors apart', () => {
    const rows = modelRows([
      { model: 'claude-opus-5-20260115', requests: 300, prompt_tokens: 9_000_000, cost: 38, output_tokens: 220_000, thinking_tokens: 0, cache_read_tokens: 7_600_000, errors: 2, subagent_requests: 20 },
      { model: 'mystery-1', requests: 4, prompt_tokens: 40_000, cost: 0, output_tokens: 900, thinking_tokens: 0, cache_read_tokens: 0, errors: 0, subagent_requests: 0 },
    ]);
    expect(rows.map((r) => [r.key, r.name, r.path, r.colour])).toEqual([
      ['claude-opus-5-20260115', 'opus-5-20260115', 'claude-opus-5-20260115', '--ar-model-opus'],
      ['mystery-1', 'mystery-1', 'mystery-1', '--ar-model-other'],
    ]);
    expect(rows[0]!.meta).toBe('300 req · 220k out · 84% cached · $38.00 · 20 by subagents');
    expect(rows[0]!.errors).toBe('2 errors');
    // A model the router could not price is not free.
    expect(rows[1]!.meta).toBe('4 req · 900 out · 0% cached · not priced');
    expect(rows[1]!.errors).toBeUndefined();
  });

  it('OV-C127: main thread vs subagents is exactly two parts, in slate and the subagent colour, with their icons', () => {
    const parts = threadParts([{ thread: 'main', requests: 3, prompt_tokens: 300, cost: 1, output_tokens: 30, cache_read_tokens: 150, agents: 0 }]);
    expect(parts.map((p) => [p.key, p.colour, p.concept, p.value])).toEqual([
      ['main', '--ar-tok', 'mainThread', 300],
      ['sub', '--ar-subagent', 'subagents', 0],
    ]);
  });

  it('OV-C128: each thread’s line gives "N req · out · cost · % cached", and for subagents how many agents, or none in scope', () => {
    const parts = threadParts([
      { thread: 'main', requests: 385, prompt_tokens: 11_000_000, cost: 37, output_tokens: 280_000, cache_read_tokens: 9_300_000, agents: 0 },
      { thread: 'subagent', requests: 27, prompt_tokens: 1_400_000, cost: 4.2, output_tokens: 30_000, cache_read_tokens: 1_100_000, agents: 6 },
    ]);
    expect(parts.map((p) => p.detail)).toEqual(['385 req · 280k out · $37.00 · 85% cached', '27 req · 30k out · $4.20 · 79% cached · 6 agents']);
    expect(threadParts([])[1]!.detail).toBe('none in scope');
  });
});

describe('the lists', () => {
  it('OV-C133: the Accounts list appears only once more than one account has served traffic in the range', () => {
    const a = (id: string, requests: number) => ({ id, label: id, active: 0, requests, prompt_tokens: 1, cost: 0, output_tokens: 0, sessions: 1, last_seen: NOW });
    expect(accountSplit([a('one', 3)])).toBeNull();
    expect(accountSplit([a('one', 3), a('idle', 0)])).toBeNull();
    expect(accountSplit([a('one', 3), a('two', 1)])?.map((x) => x.id)).toEqual(['one', 'two']);
  });

  it('OV-C134: an account row shows its icon, its label and router id, "active" while it serves, and its sessions, requests, output and last seen', () => {
    const [row] = accountRows([{ id: 'secondary', label: 'secondary@example.test', active: 1, requests: 300, prompt_tokens: 9_000_000, cost: 30, output_tokens: 200_000, sessions: 2, last_seen: NOW - 2 * MIN }], NOW);
    expect(row).toMatchObject({ concept: 'account', name: 'secondary@example.test', path: 'secondary', active: true, value: 9_000_000, figure: '9.00M' });
    expect(row!.meta).toBe('2 sessions · 300 req · 200k out · 2 min ago');
  });

  it('OV-C136: a project with no recorded path is listed as "(unresolved)" and scopes like any other', () => {
    const rows = projectRows([project(null, 1_000)], NOW);
    expect(rows[0]).toMatchObject({ key: '(unresolved)', name: '(unresolved)', path: undefined });
    expect(projectKeyOf({ project: null })).toBe('(unresolved)');
  });

  it('OV-C214: a project is scoped by its project key and shows its directories with their machines', () => {
    const repository = 'gitlab.example.com/acme/ui-kit-web';
    const remote = { name: 'origin', url: 'git@gitlab.example.com:acme/ui-kit-web.git', repository };
    const row = project(repository, 9_000_000, {
      repository,
      directories: [
        { cwd: '/Users/sample/code/ui-kit-web', machine: { id: 'm-studio', label: 'studio' }, remotes: [remote] },
        { cwd: '/Users/sample/code/ui-kit-web-lane-2', machine: { id: 'm-studio', label: 'studio' }, remotes: [remote] },
        { cwd: 'C:\\Users\\sample\\ui-kit-web', machine: { id: 'm-win', label: ' ' }, remotes: [remote] },
      ],
    });
    // Keyed, listed, linked and read by its key, the repository, and named by its last segment.
    expect([projectKeyOf(row), projectKey(row)]).toEqual([repository, repository]);
    expect(projectRows([row], NOW)[0]).toMatchObject({ key: repository, name: 'ui-kit-web', path: repository });
    expect(new URL(routes.project(projectKey(row)), 'http://console.test').searchParams.get('project')).toBe(repository);
    const paths = snapshotPaths(scoped({ project: repository }), null, 0);
    for (const read of [paths.summary, paths.sessions, paths.tools, paths.requests]) expect(new URL(read, 'http://console.test').searchParams.get('project')).toBe(repository);
    // Where it ran: the repository, then each directory on its machine; a blank label names none.
    expect(projectPlaces(repository, [row])).toEqual({
      repository,
      directories: [
        { machine: 'studio', path: '/Users/sample/code/ui-kit-web', label: 'studio · /Users/sample/code/ui-kit-web' },
        { machine: 'studio', path: '/Users/sample/code/ui-kit-web-lane-2', label: 'studio · /Users/sample/code/ui-kit-web-lane-2' },
        { machine: null, path: 'C:\\Users\\sample\\ui-kit-web', label: 'C:\\Users\\sample\\ui-kit-web' },
      ],
    });
    // A local router's project with no remote is its directory, which is said as the path alone.
    expect(projectPlaces('/code/agent-router', [], [project('/code/agent-router', 1)])).toEqual({
      repository: null,
      directories: [{ machine: null, path: '/code/agent-router', label: '/code/agent-router' }],
    });
    // Nothing scoped, a row in neither list, and an unresolved project with nowhere to say: nothing.
    expect([projectPlaces(null, [row]), projectPlaces('gitlab.example.com/acme/other', [row]), projectPlaces('(unresolved)', [project(null, 1)])]).toEqual([null, null, null]);
  });

  it('OV-C137: a session row: its first eight characters in monospace, its chips, the subagent share and its figures', () => {
    const [row] = sessionRows([session(S1, { started_at: NOW - 3 * HOUR, last_seen_at: NOW - 2 * MIN, subagent_tokens: 600_000 })], UNSCOPED, NOW);
    expect(row).toMatchObject({ key: S1, name: 'aaaaaaaa', mono: true, concept: 'session', sub: 600_000, subFigure: '10% subagents', errors: '1 error' });
    expect(row!.chipsOf).toEqual({
      branch: 'dev',
      project: { name: 'agent-router', path: '/code/agent-router' },
      client: { name: 'VS Code', entrypoint: 'claude-vscode, claude-cli' },
      families: [{ family: 'opus', model: 'claude-opus-5-20260115' }],
    });
    expect(row!.meta).toMatch(/^200 req · \$20\.00 · 150k out · 3 subagents · started .+ · 2h 58m$/);
    // Its project chip only when no project is scoped.
    expect(sessionRows([session(S1)], scoped({ project: '/code/agent-router' }), NOW)[0]!.chipsOf.project).toBeNull();
  });

  it('OV-C138: the Sessions caption names the scope and the count, and "200 most recent" at the cap', () => {
    expect(sessionsCaption(14, UNSCOPED)).toBe('all projects · 14 sessions · click one to scope the page');
    expect(sessionsCaption(1, scoped({ project: '/code/northstar' }))).toBe('in northstar · 1 session · click one to scope the page');
    expect(sessionsCaption(200, scoped({ session: S1 }))).toBe('all projects · 200 most recent · click the highlighted session again to clear');
  });

  it('OV-C140: "not yet attributed" counts under Projects, and under Sessions when no project is scoped, and is absent when there are none or a session is scoped', () => {
    expect(unattributed(4, 'project', UNSCOPED)?.meta).toBe('4 requests · no project recorded yet');
    expect(unattributed(1, 'session', UNSCOPED)?.meta).toBe('1 request · no session recorded yet');
    expect(unattributed(4, 'session', scoped({ project: '/code/a' }))).toBeNull();
    expect(unattributed(4, 'project', scoped({ project: '/code/a' }))).not.toBeNull();
    expect([unattributed(0, 'project', UNSCOPED), unattributed(4, 'project', scoped({ session: S1 }))]).toEqual([null, null]);
  });

  it("OV-C141: counts only the router's own count of requests no session holds, never what a capped list left out", () => {
    // Two hundred sessions listed of many more: nothing is subtracted, the router's count stands.
    expect(unattributed(3, 'session', UNSCOPED)?.meta).toBe('3 requests · no session recorded yet');
    expect(unattributed(null, 'session', UNSCOPED)).toBeNull();
  });

  it('OV-C142: the row explains itself beside it, and says these requests are in the totals above', () => {
    expect(unattributed(4, 'project', UNSCOPED)?.why).toBe(
      'Claude Code labels a request with its session id; the project comes from the transcript file, which can appear a moment later. These requests are counted in the totals above.',
    );
  });

  it('OV-C143: tool output has one row per tool by estimated tokens, "N results · ≈X each · ≥$cost", and "N failed" in the error mark', () => {
    const rows = toolOutputRows([{ name: 'Read', results: 120, chars: 4_800_000, est_tokens: 1_200_000, max_tokens: 40_000, est_cost_once: 18, errors: 2 }]);
    expect(rows[0]).toMatchObject({ key: 'Read', concept: 'tools', colour: '--ar-ctx-tools', value: 1_200_000, figure: '1.20M', errors: '2 failed' });
    expect(rows[0]!.meta).toBe('120 results · ≈10k each · ≥$18.00 if paid once as input · 4.80M characters · largest 40k');
  });

  it('OV-C144: tool calls have one row per tool by calls, the count exact, the subagents’ calls laid over and labelled, and "N calls · in M requests"', () => {
    const [row] = toolCallRows([{ name: 'Read', calls: 1_234, requests: 300, subagent_calls: 200 }]);
    expect(row).toMatchObject({ value: 1_234, sub: 200, figure: (1_234).toLocaleString(), subFigure: '200 by subagents', meta: `${(1_234).toLocaleString()} calls · in 300 requests` });
  });

  it('OV-C145: every empty panel says which empty it is: nothing recorded at all with how to start, nothing in the range, nothing in this scope', () => {
    expect(emptyWords('projects', UNSCOPED)).toBe('No projects yet — point Claude Code at this router and run it.');
    expect(emptyWords('projects', scoped({ range: '24h' }))).toBe('No project sent a request in the last 24h.');
    expect(emptyWords('sessions', scoped({ range: '7d' }))).toBe('No session ran in the last 7d.');
    expect(emptyWords('toolOutput', scoped({ account: 'primary' }))).toBe('No tool results in this scope.');
    expect(emptyWords('read', scoped({ project: '/code/a', range: '1h' }))).toBe('Nothing read in this scope in the last 1h.');
    // "Yet" only where nothing at all is recorded.
    expect(emptyWords('toolCalls', scoped({ range: '30d' }))).not.toMatch(/yet/);
  });
});
