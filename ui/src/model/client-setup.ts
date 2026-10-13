import type { Provider } from './settings';
import type { RegistryCapabilities } from './provider-registry';

export type SetupClientId = 'claude-code' | 'codex' | 'opencode';

export interface SetupClient {
  readonly id: SetupClientId;
  readonly name: string;
  readonly protocol: 'messages' | 'responses';
  readonly installed: boolean;
  readonly version: string | null;
  readonly configured: boolean;
  readonly managed: boolean;
  readonly config_path: string | null;
  readonly can_apply: boolean;
}

export interface SetupModel {
  readonly id: string;
  readonly provider: Provider;
  readonly display_name: string;
  readonly account_id: string | null;
  readonly available: boolean;
  readonly service_id?: string;
  readonly native_protocol?: 'messages' | 'responses';
  readonly capabilities?: RegistryCapabilities;
}

export interface ClientSetup {
  readonly mode: 'local' | 'server';
  readonly url: string;
  readonly clients: readonly SetupClient[];
  readonly models: readonly SetupModel[];
  /// The machine whose clients these are, when a hosted router read them through its agent.
  readonly machine?: string;
}

export interface ClientSetupInput {
  readonly model: string;
  readonly url: string;
  readonly token?: string;
  readonly project?: string;
  readonly revision?: string;
}

export interface ClientCheck {
  readonly id: string;
  readonly label: string;
  readonly status: 'pass' | 'warning' | 'fail';
  readonly detail: string;
}

export interface ClientPreview {
  readonly ok: true;
  readonly client: SetupClientId;
  readonly model: string;
  readonly provider: Provider;
  readonly account_id: string | null;
  readonly protocol: 'messages' | 'responses';
  readonly translated: boolean;
  readonly can_apply: boolean;
  readonly path: string | null;
  readonly format: 'json' | 'toml';
  readonly content: string;
  readonly warnings: readonly string[];
  readonly checks: readonly ClientCheck[];
  readonly revision: string;
  readonly connection_tested: false;
  readonly service_id?: string;
  readonly native_protocol?: 'messages' | 'responses';
  readonly capabilities?: RegistryCapabilities;
  readonly catalogue_content?: string | null;
  readonly catalogue_path?: string | null;
  readonly catalogue_qualified_version?: string | null;
}

export type ClientChecked = Omit<ClientPreview, 'content'>;

export interface ClientApplied {
  readonly ok: true;
  readonly client: SetupClientId;
  readonly path: string;
  readonly changed: boolean;
  readonly warnings: readonly string[];
}

export interface ClientRestored {
  readonly ok: true;
  readonly client: SetupClientId;
  readonly path: string;
  readonly changed: boolean;
  readonly preserved_edits: boolean;
}

export function clientLimits(client: SetupClientId, provider: Provider | undefined, nativeProtocol?: 'messages' | 'responses', capabilities?: RegistryCapabilities): readonly string[] {
  const responses = nativeProtocol ? nativeProtocol === 'responses' : provider === 'openai';
  if (client === 'codex') return [
    ...((provider && !responses) || capabilities?.compaction === false ? ['Codex conversation compaction is unavailable on this route. Start a new session when the conversation gets long.'] : []),
    'Codex web search is disabled in the generated configuration.',
    'Start a new session when switching providers. Saved private reasoning and provider-specific conversation history may not translate.',
  ];
  return responses ? ['This route uses Responses, which does not provide the Messages token-counting endpoint used by some client features. Messages stop sequences are also unavailable on this route.'] : [];
}

export const clientFilename = (client: SetupClientId, format: ClientPreview['format']): string =>
  client === 'claude-code' ? 'settings.local.json' : client === 'codex' ? 'config.toml' : `opencode.${format}`;
