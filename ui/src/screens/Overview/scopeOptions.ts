// New for agent-router: the scope pickers' rows (assets/index.html, `menuRows`), with no DOM: "All …" first,
// then a row per account, project or session with its name, its second line and its figures (OV-C16).
import type { ScopeOption } from '~/components/ScopePicker';
import type { AccountSplitRow, SessionRow } from '~/data/overview';
import type { ProjectRow } from '~/data/router';
import { compact, count, id8, projectName, tod, usd } from '~/model/format';
import { sessionsHeld, type Scope } from '~/model/scope';
import { providerOf, type Provider } from '~/model/settings';
import { isActive, projectKeyOf, providersOf, providerWords } from './panels';

/// The key of "All …" in every picker.
export const ALL = '*';

const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

/// A picker's figures: "1.20M · 840 req · $41.20".
const figures = (r: { prompt_tokens: number | null; requests: number | null; cost: number | null }) => `${compact(r.prompt_tokens)} · ${count(r.requests)} req · ${usd(r.cost)}`;

/// The account picker: every account that served in the range, by its label with its router id on
/// the second line (OV-C16, OV-C17), and its provider, named there — "active for" it while the
/// account signs that provider's new requests — and marked before the name (OV-C231). Never narrowed
/// by the level it picks (OV-C131).
export function accountOptions(list: readonly AccountSplitRow[]): (ScopeOption & { readonly provider?: Provider })[] {
  const all = list.reduce((a, x) => a + n(x.requests), 0);
  return [
    { key: ALL, name: 'All accounts', meta: `${count(all)} req` },
    ...list.map((a) => {
      const provider = providerOf(a);
      return { key: a.id, name: a.label || a.id, provider, sub: `${a.id} · ${providerWords(provider, isActive(a))}`, meta: figures(a) };
    }),
  ];
}

/// The project picker: each project by its folder's name, its path giving up its head rather than
/// its tail (OV-C16). "All projects" counts the requests the top counts.
export function projectOptions(list: readonly ProjectRow[], requests: number | null): ScopeOption[] {
  return [
    { key: ALL, name: 'All projects', meta: `${count(requests)} req` },
    ...list.map((p) => ({ key: projectKeyOf(p), name: projectName(p.project), sub: p.project ?? '', path: true, meta: figures(p) })),
  ];
}

/// The session picker: each session by its first eight characters, with its branch, its project
/// unless one is scoped, and its start (OV-C16), and the providers of its requests marked before it
/// (SE-C242).
export function sessionOptions(list: readonly SessionRow[], scope: Scope, now: number): (ScopeOption & { readonly providers?: readonly Provider[] })[] {
  return [
    { key: ALL, name: scope.project ? `All sessions in ${projectName(scope.project)}` : 'All sessions', meta: sessionsHeld(list.length) },
    ...list.map((x) => ({
      key: x.id,
      name: id8(x.id),
      providers: providersOf(x),
      mono: true,
      sub: [x.git_branch, scope.project ? null : projectName(x.project), tod(x.started_at, now)].filter(Boolean).join(' · '),
      meta: figures(x),
    })),
  ];
}
