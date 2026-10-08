// New for agent-router: the Library screen's reads and writes (docs/library.md, "API"; src/library/api.rs,
// push.rs), typed by hand because the router has no OpenAPI spec. Everything goes through ~/data/http,
// the one fetch layer, so a refusal and a timeout read here as on every other screen (SH-C23).

import { getJson, getText, postBytes, postJson } from '~/data/http';
import { Refused } from '~/lib/errors';
import type { ItemDetail, ItemRow, Kind, Told, Version } from '~/model/library';

export const libraryKeys = {
  all: ['library'] as const,
  list: ['library', 'list'] as const,
  item: (kind: string, name: string) => ['library', 'item', kind, name] as const,
  file: (kind: string, name: string, version: number, path: string) => ['library', 'file', kind, name, version, path] as const,
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
