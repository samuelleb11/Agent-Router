// New for agent-router: Settings → Router, against the router's answers (docs/web-console-contracts.md,
// "Router").
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Auth } from '~/model/settings';
import { json, renderScreen, unreachable } from '~/test-utils';
import RouterPage from './Router';
import { HOSTED, LOCAL, ok, stubRouter, type Answer } from './testing';

afterEach(() => vi.unstubAllGlobals());

const HEALTH = { ok: true, pid: 4242, version: '0.9.0', build: 'f00dfeed' };

function show(auth: Auth = LOCAL, more: Record<string, Answer> = {}) {
  const router = stubRouter({ '/api/auth': ok(auth), '/_router/health': ok(HEALTH), ...more });
  const view = renderScreen(<RouterPage />, { path: '/settings/router', at: '/settings/router' });
  return { ...router, ...view };
}

const facts = () => screen.findByLabelText('This router');
const snippet = () => screen.findByRole('textbox', { name: 'What to add to .claude/settings.local.json' });

describe('Router', () => {
  it('ST-C95, ST-C5: the page states where it is read from, the version and locally the process id, reading the health afresh each time it opens', async () => {
    const r = show();
    const list = await facts();
    await waitFor(() => expect(within(list).getByText('Version').nextElementSibling).toHaveTextContent('0.9.0'));
    expect(within(list).getByText('Listening on').nextElementSibling).toHaveTextContent(location.host);
    expect(within(list).getByText('Process').nextElementSibling).toHaveTextContent('4242');
    expect(r.gets('/_router/health')).toHaveLength(1);
    r.unmount();
    renderScreen(<RouterPage />, { path: '/settings/router', at: '/settings/router' });
    await waitFor(() => expect(r.gets('/_router/health')).toHaveLength(2));
  });

  it('ST-C95: a failed health read is said, never kept as an empty answer; a hosted router shows no process id', async () => {
    show(LOCAL, { '/_router/health': unreachable });
    expect(await screen.findByText(/^Could not read the router’s health/)).toHaveTextContent('Could not reach the router.');
    expect(within(await facts()).getByText('Version').nextElementSibling).toHaveTextContent('—');
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show(HOSTED);
    const hosted = await facts();
    await waitFor(() => expect(within(hosted).getByText('Version').nextElementSibling).toHaveTextContent('0.9.0'));
    expect(within(hosted).queryByText('Process')).toBeNull();
  });

  it('ST-C96, ST-C97: the snippet points at the page’s own origin, drawn as for this machine until /api/auth answers, and redrawn with the password line when the answer asks for it', async () => {
    let answer: (r: Response) => void = () => {};
    show(LOCAL, { '/api/auth': () => new Promise<Response>((r) => (answer = r)) });
    const before = await snippet();
    expect(before).toHaveTextContent(`{ "env": { "ANTHROPIC_BASE_URL": "${location.origin}" } }`);
    expect(before).not.toHaveTextContent('ANTHROPIC_AUTH_TOKEN');
    answer(json({ ...LOCAL, from_loopback: false }));
    await waitFor(async () => expect(await snippet()).toHaveTextContent('"ANTHROPIC_AUTH_TOKEN": "<the router password>"'));
    expect(await snippet()).toHaveTextContent(`"ANTHROPIC_BASE_URL": "${location.origin}"`);
  });

  it('ST-C98: the snippet says where it goes and has a Copy button that says Copied, or to select it by hand when the clipboard refuses', async () => {
    show();
    expect(await screen.findByText('.claude/settings.local.json')).toBeInTheDocument();
    const writeText = vi.fn(async (_text: string): Promise<void> => {
      throw new DOMException('denied', 'NotAllowedError');
    });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    try {
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
      expect(await screen.findByText('Copying failed — select the snippet and copy it by hand.')).toBeInTheDocument();
      writeText.mockImplementation(async () => {});
      fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
      expect(await screen.findByText('Copied')).toBeInTheDocument();
      expect(writeText).toHaveBeenLastCalledWith(`{ "env": { "ANTHROPIC_BASE_URL": "${location.origin}" } }`);
    } finally {
      Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it('ST-C99, ST-C102: the page says where the router’s own settings come from: environment variables at startup locally, the deployment on a hosted router', async () => {
    show();
    expect(await screen.findByText(/^Everything above is stored locally/)).toHaveTextContent(
      "Everything above is stored locally. The router's own settings (port, database location, upstream, body cap) are environment variables read at startup — see the README.",
    );
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
    show(HOSTED);
    const hosted = await screen.findByText(/^Everything above is kept in the router’s data directory/);
    expect(hosted).toHaveTextContent('Its own settings (port, database location, upstream, body cap) are set by the deployment and read at startup.');
    expect(document.body).not.toHaveTextContent('stored locally');
  });
});
