// New for agent-router: Settings → Forges and Settings → Library, with no DOM and no React
// (docs/web-console-contracts.md, "Forges and the Library"; docs/library.md, "Forges and sources",
// "Forge webhooks", "Agents publishing"). What the router answers for its forges, sources and
// publish tokens, and those answers in words.

export type ForgeKind = 'gitlab' | 'github';
export const FORGE_KINDS: readonly { readonly kind: ForgeKind; readonly label: string }[] = [
  { kind: 'gitlab', label: 'GitLab' },
  { kind: 'github', label: 'GitHub' },
];

/// A forge as `GET /api/library/forges` lists it: never its token, only whether one is kept
/// (src/library/sources.rs, `ForgeRow`).
export interface ForgeRow {
  readonly key: string;
  readonly kind: ForgeKind;
  readonly base_url: string;
  readonly token_present: boolean;
  readonly sources: number;
  readonly created_at: number;
}

export interface ImportLine {
  readonly kind: string;
  readonly name: string;
  readonly path: string;
  readonly outcome: 'new' | 'unchanged' | 'refused' | 'failed';
  readonly version?: number;
  readonly state?: string;
  readonly why?: string;
}

/// What importing a source did (src/library/sources.rs, `Imported`).
export interface Imported {
  readonly source: string;
  readonly commit: string;
  readonly fetched: boolean;
  readonly items: readonly ImportLine[];
  readonly gone: readonly string[];
}

/// A source as `GET /api/library/sources` lists it: never its webhook secret, only whether it has
/// one.
export interface SourceRow {
  readonly key: string;
  readonly forge: string;
  readonly repository: string;
  readonly ref: string;
  readonly path: string;
  readonly live: boolean;
  readonly head: string | null;
  readonly checked_at: number | null;
  readonly imported_at: number | null;
  /// The last import's result, or the forge's refusal as `{ok: false, error}`.
  readonly last_result: Imported | { readonly ok: false; readonly error: string } | null;
  readonly hook: boolean;
  readonly created_at: number;
}

/// A publish token as `GET /api/library/tokens` lists it: never its value.
export interface TokenRow {
  readonly id: string;
  readonly name: string;
  readonly kinds: readonly string[];
  readonly items: readonly string[];
  readonly live: boolean;
  readonly created_at: number;
  readonly last_used_at: number | null;
  readonly revoked_at: number | null;
}

export const kindLabel = (kind: string): string => FORGE_KINDS.find((k) => k.kind === kind)?.label ?? kind;

/// A forge's check, as a heading over the router's own words (ST-C107).
export const CHECKED: Readonly<Record<string, string>> = {
  ok: 'The forge answered',
  unauthorised: 'The forge refused the token',
  insufficient_scope: 'The token may not read this',
  not_found: 'The forge has no such repository',
  unreachable: 'The forge could not be reached',
  unexpected: 'The forge answered something unexpected',
};

/// What an import came to, counted (ST-C111): "3 new, 2 unchanged, 1 refused at 0123abcd".
export function importWords(imported: Imported): string {
  if (!imported.fetched) return `Nothing to fetch: the ref is still at ${imported.commit.slice(0, 8)}.`;
  const count = (outcome: ImportLine['outcome']) => imported.items.filter((l) => l.outcome === outcome).length;
  const parts = [`${count('new')} new`, `${count('unchanged')} unchanged`];
  if (count('refused')) parts.push(`${count('refused')} refused`);
  if (count('failed')) parts.push(`${count('failed')} failed`);
  const gone = imported.gone.length ? `; no longer in the repository: ${imported.gone.join(', ')}` : '';
  return `Imported at ${imported.commit.slice(0, 8)}: ${parts.join(', ')}${gone}.`;
}

/// What a source's last import came to, or that it has not run, or the forge's refusal (ST-C109).
export function lastWords(source: Pick<SourceRow, 'last_result'>): string {
  const last = source.last_result;
  if (last === null) return 'Never imported yet.';
  if ('error' in last) return `The forge said no: ${last.error}`;
  return importWords(last);
}

/// What a token may publish (ST-C113).
export function scopeWords(token: Pick<TokenRow, 'kinds' | 'items'>): string {
  if (token.items.length) return token.items.join(', ');
  if (token.kinds.length) return token.kinds.map((k) => `any ${k}`).join(', ');
  return 'anything';
}

/// A key a forge or a source is known by: the same shape as an item's name.
export const isKey = (key: string): boolean => /^[a-z0-9]+(-[a-z0-9]+)*$/.test(key) && key.length <= 64;
