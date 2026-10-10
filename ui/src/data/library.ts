// New for agent-router: the Library screen's reads and writes (docs/library.md, "API"; src/library/api.rs,
// push.rs), typed by hand because the router has no OpenAPI spec. Everything goes through ~/data/http,
// the one fetch layer, so a refusal and a timeout read here as on every other screen (SH-C23).

import { deleteJson, getJson, getText, postBytes, postJson, putJson } from '~/data/http';
import { Refused } from '~/lib/errors';
import type { ForgeRow, Imported, SourceRow, TokenRow } from '~/model/forges';
import type { ItemDetail, ItemRow, Kind, Told, Usage, Version } from '~/model/library';

export const libraryKeys = {
  all: ['library'] as const,
  list: ['library', 'list'] as const,
  item: (kind: string, name: string) => ['library', 'item', kind, name] as const,
  file: (kind: string, name: string, version: number, path: string) => ['library', 'file', kind, name, version, path] as const,
  usage: ['library', 'usage'] as const,
};

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const at = (kind: string, name: string) => `/api/library/items/${encodeURIComponent(kind)}/${encodeURIComponent(name)}`;

/// `GET /api/library`: every item. A reply without a list is a failed read, never an empty
/// Library (LB-C13).
export async function readLibrary(signal?: AbortSignal): Promise<ItemRow[]> {
  const body = await getJson<unknown>('/api/library', { signal });
  if (!isRecord(body) || !Array.isArray(body['items'])) throw new Refused(200, 'The router answered something that is not the Library.');
  return body['items'].filter(isRecord) as unknown as ItemRow[];
}

/// `GET /api/library/usage`: every item's use over 30 days, in the viewer's days, and what the
/// Library's skills add to every turn. A reply without its items is a failed read, never "not
/// used" (LB-C17).
export async function readUsage(signal?: AbortSignal, tz = -new Date().getTimezoneOffset()): Promise<Usage> {
  const body = await getJson<unknown>(`/api/library/usage?tz=${tz}`, { signal });
  if (!isRecord(body) || !Array.isArray(body['items']) || !Array.isArray(body['days'])) {
    throw new Refused(200, 'The router answered something that is not the Library’s usage.');
  }
  return body as unknown as Usage;
}

/// `GET /api/library/items/{kind}/{name}`: one item, its versions and its files.
export const readItem = (kind: string, name: string, signal?: AbortSignal) => getJson<ItemDetail>(at(kind, name), { signal });

/// One file of one version, as text.
export const readFile = (kind: string, name: string, version: number, path: string, signal?: AbortSignal) =>
  getText(`${at(kind, name)}/versions/${version}/files/${path.split('/').map(encodeURIComponent).join('/')}`, { signal });

interface Published {
  readonly unchanged: boolean;
  readonly version: Version;
}

/// An upload from the console: the bytes, as what they are. A Library version may be 8 MiB, so a
/// slow link gets a minute.
export const upload = (kind: Kind, name: string, body: Blob, contentType: string) =>
  postBytes<Published>(`${at(kind, name)}/versions`, body, contentType, { timeoutMs: 60_000 });

export const makeLive = (kind: string, name: string, version: number) => postJson<{ version: Version }>(`${at(kind, name)}/versions/${version}/live`, {});
export const reject = (kind: string, name: string, version: number) => postJson<{ version: Version }>(`${at(kind, name)}/versions/${version}/reject`, {});
export const archive = (kind: string, name: string) => postJson<{ ok: true }>(`${at(kind, name)}/archive`, {});

/// `POST /api/library/sync`: every machine, or the one named. A sync downloads what changed, so
/// the router may take a while to answer for a slow machine.
export const syncNow = (machine?: string) =>
  postJson<{ machines: Told[] }>('/api/library/sync', machine ? { machine } : {}, { timeoutMs: 130_000 });

// ---------- Settings: forges, sources, tokens, machine publishes ----------


export const settingKeys = {
  forges: ['library', 'forges'] as const,
  sources: ['library', 'sources'] as const,
  tokens: ['library', 'tokens'] as const,
  settings: ['library', 'settings'] as const,
};

export const readForges = (signal?: AbortSignal) => getJson<{ forges: ForgeRow[] }>('/api/library/forges', { signal }).then((b) => b.forges);
/// `token`: absent keeps the kept one, `null` removes it, a string replaces it.
export const putForge = (key: string, body: { kind: string; base_url: string; token?: string | null }) =>
  putJson<{ ok: true }>(`/api/library/forges/${encodeURIComponent(key)}`, body);
export const deleteForge = (key: string) => deleteJson<{ ok: true }>(`/api/library/forges/${encodeURIComponent(key)}`);
export const checkForge = (key: string) =>
  postJson<{ result: string; words: string }>(`/api/library/forges/${encodeURIComponent(key)}/check`, {}, { timeoutMs: 70_000 });

export const readSources = (signal?: AbortSignal) => getJson<{ sources: SourceRow[] }>('/api/library/sources', { signal }).then((b) => b.sources);
export const putSource = (key: string, body: { forge: string; repository: string; ref: string; path: string; live: boolean }) =>
  putJson<{ ok: true }>(`/api/library/sources/${encodeURIComponent(key)}`, body);
export const deleteSource = (key: string) => deleteJson<{ ok: true }>(`/api/library/sources/${encodeURIComponent(key)}`);
/// An import fetches a repository's archive: a big one on a slow forge gets two minutes.
export const importSource = (key: string) =>
  postJson<{ imported: Imported }>(`/api/library/sources/${encodeURIComponent(key)}/import`, {}, { timeoutMs: 130_000 });
export const newSecret = (key: string) =>
  postJson<{ secret: string; path: string; url: string | null }>(`/api/library/sources/${encodeURIComponent(key)}/secret`, {});

export const readTokens = (signal?: AbortSignal) => getJson<{ tokens: TokenRow[] }>('/api/library/tokens', { signal }).then((b) => b.tokens);
export const createToken = (body: { name: string; kinds: string[]; items: string[]; live: boolean }) =>
  postJson<{ id: string; name: string; token: string }>('/api/library/tokens', body);
export const revokeToken = (id: string) => deleteJson<{ ok: true }>(`/api/library/tokens/${encodeURIComponent(id)}`);

export const readLibrarySettings = (signal?: AbortSignal) => getJson<{ machine_publishes: 'live' | 'approval' }>('/api/library/settings', { signal });
export const setMachinePublishes = (live: boolean) =>
  putJson<{ machine_publishes: string }>('/api/library/settings', { machine_publishes: live ? 'live' : 'approval' });
