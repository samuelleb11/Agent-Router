// Native client identity is separate from the provider that serves a model.
export const CODEX = 'codex';
export const CODEX_WATCHED = 'This Codex session is watched from its local transcript. Send messages and answer approvals in the terminal or IDE that owns it.';

export function isCodex(id: string, row?: { readonly client?: string | null } | null): boolean {
  return row?.client === CODEX || id.startsWith('codex:');
}
