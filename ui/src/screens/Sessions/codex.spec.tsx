import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CODEX_WATCHED } from '~/model/session';
import { FakeEventSource } from './fakes';
import { json, renderSessions, session, streamOf, stubRouter, working } from './kit';

beforeEach(() => FakeEventSource.reset());
afterEach(() => vi.unstubAllGlobals());

it('names a watched Codex session, holds its controls and avoids Claude wrapper advice', async () => {
  const id = 'codex:12345678-0000-4000-8000-000000000001';
  const router = stubRouter((req) => {
    if (req.path === '/api/live/sessions') return json([session(id, { client: 'codex', wrapped: false, hosted: false })]);
    if (req.path === '/api/live/stats') return json({ model: 'gpt-5.4', total: 100, output: 20, turns: 1 });
    return undefined;
  });
  renderSessions(`/live?session=${id}`);
  await waitFor(() => streamOf(id));
  act(() => { streamOf(id).open().status(working()); });
  await waitFor(() => expect(screen.getByRole('textbox', { name: 'Message to this session' })).toBeDisabled());
  expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Side question' })).toBeDisabled();
  const facts = within(screen.getByRole('region', { name: 'This session’s totals and settings' }));
  expect(facts.getByText('Codex')).toBeInTheDocument();
  expect(facts.getByText(CODEX_WATCHED)).toBeInTheDocument();
  expect(facts.queryByText(/installing the wrapper/)).toBeNull();
  expect(document.querySelector('[data-client-fact] [data-provider]')).toBeNull();
  expect(router.asked.filter((req) => req.method === 'POST')).toEqual([]);
});

it('enables the managed Codex composer and native setting controls, then sends to its own thread', async () => {
  const id = 'codex:managed-thread';
  const router = stubRouter((req) => {
    if (req.path === '/api/live/sessions') return json([session(id, { client: 'codex', wrapped: true, hosted: true })]);
    if (req.path === '/api/live/stats') return json({ model: 'gpt-5.4', effort: 'low', mode: 'default', total: 100, output: 20, turns: 1 });
    return undefined;
  });
  renderSessions(`/live?session=${id}`);
  await waitFor(() => streamOf(id));
  act(() => { streamOf(id).open().status(working()); });
  const textbox = screen.getByRole('textbox', { name: 'Message to this session' });
  await waitFor(() => expect(textbox).toBeEnabled());
  const facts = within(screen.getByRole('region', { name: 'This session’s totals and settings' }));
  expect(await facts.findByRole('combobox', { name: 'Model' })).toBeEnabled();
  expect(facts.getByRole('combobox', { name: 'Effort' })).toBeEnabled();
  expect(facts.getByRole('combobox', { name: 'Mode' })).toBeEnabled();
  expect(facts.queryByText(CODEX_WATCHED)).toBeNull();
  fireEvent.change(textbox, { target: { value: 'Continue the managed thread.' } });
  fireEvent.click(screen.getByRole('button', { name: /^Send$/ }));
  await waitFor(() => expect(router.asked.some(req => req.path === '/api/live/send' && req.body?.session === id && req.body?.text === 'Continue the managed thread.')).toBe(true));
  expect(screen.getByRole('button', { name: 'Side question' })).toBeDisabled();
});
