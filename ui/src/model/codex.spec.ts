// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { client, clientLabel, id8, productName } from './format';
import { controlsOf, drives, rowClient, rowProvider, CODEX_WATCHED } from './session';
import { identityNote, providerName, removable } from './settings';
import { keyProblem, sourceWord } from '~/screens/Settings/rules';

describe('Codex clients and accounts', () => {
  it('names requests and machines consistently', () => {
    expect(clientLabel('codex-cli')).toBe('Codex CLI');
    expect(client('codex_vscode/0.114.0')).toBe('Codex VS Code');
    expect(client('codex_cli_rs/0.114.0')).toBe('Codex CLI');
    expect(productName('codex')).toBe('Codex');
    expect(id8('codex:12345678-1234')).toBe('12345678');
    expect(clientLabel('opencode')).toBe('OpenCode');
    expect(client('opencode/1.19.0 ai-sdk/provider-utils/4.0.46 runtime/bun/1.3.14')).toBe('OpenCode');
    expect(id8('opencode:ses_ed6b896c6ffe7YY9vO4bK3uEJq')).toBe('ed6b896c');
    expect(providerName('openai')).toBe('OpenAI');
    expect(sourceWord('codex')).toBe('Codex login');
  });

  it('holds every watched session control, including before the list arrives', () => {
    const row = { session_id: 'codex:1234', client: 'codex', wrapped: false, hosted: false };
    expect(drives(row)).toBe(false);
    expect(rowClient(row)).toBe('Codex');
    expect(rowProvider(row)).toBeNull();
    for (const r of [row, null]) {
      expect(controlsOf(row.session_id, r)).toMatchObject({ held: CODEX_WATCHED, aside: CODEX_WATCHED, question: CODEX_WATCHED, effort: CODEX_WATCHED });
    }
  });

  it('enables controls only for a managed reachable Codex thread', () => {
    const row = { session_id: 'codex:managed', client: 'codex', wrapped: true, hosted: true };
    expect(drives(row)).toBe(true);
    expect(drives({ ...row, wrapped: false })).toBe(false);
    expect(controlsOf(row.session_id, row)).toMatchObject({ held: null, effort: null, question: null });
    expect(controlsOf(row.session_id, null).held).toBe(CODEX_WATCHED);
    expect(controlsOf(row.session_id, row).aside).toContain('not available');
  });

  it('keeps the discovered login local and validates OpenAI API keys', () => {
    expect(removable({ source: 'codex' }, {})).toBe(false);
    expect(removable({ source: 'codex' }, { mode: 'server' })).toBe(true);
    expect(identityNote({ source: 'codex' }, {})).toContain('Codex login');
    expect(keyProblem('openai', 'sk-proj-example')).toBeNull();
    expect(keyProblem('openai', 'sk-ant-oat01-example')).toContain('Anthropic');
    expect(keyProblem('openai', 'not-a-key')).toContain('starting sk-');
  });
});
