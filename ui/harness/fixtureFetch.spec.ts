// Copied from Northstar ui/harness/fixtureFetch.spec.ts at e9c1589.
// Changed: the router's reads, and the console rendered whole on them.
import { createElement } from 'react';
import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectRow } from '~/data/router';
import { fixtureServer, type FixtureServer } from './fixtureFetch';
import { HEALTH, PROJECTS } from './fixtures';

/// The harness answers the console's reads from fixtures, so a screen can be seen and probed
/// with no router. Two promises are held here: every read the shell makes is answered, and
/// anything else fails where a reader will see it, never quietly.

const BASE = 'http://harness.invalid';
const get = (h: FixtureServer, path: string, init?: RequestInit) => h.fetch(`${BASE}${path}`, init);
const body = async <T>(r: Response): Promise<T> => (await r.json()) as T;

describe('fixtureServer', () => {
  let errors: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  });
  afterEach(() => {
    errors.mockRestore();
    vi.unstubAllGlobals();
  });

  it('answers every read the shell makes, and the console draws from them', async () => {
    const h = fixtureServer();
    vi.stubGlobal('fetch', h.fetch);
    // The router reads the address once, when the route table is made, so it is set first.
    window.history.pushState({}, '', '/');
    const { App } = await import('~/app/App');
    render(createElement(App));

    // The rail's projects are the fixture's, by name; the screen is a lazy chunk the first test
    // in a run waits on.
    expect(await screen.findByRole('link', { name: 'agent-router' }, { timeout: 10_000 })).toBeInTheDocument();
    expect(await screen.findByRole('heading', { name: 'Overview' }, { timeout: 10_000 })).toBeInTheDocument();
    expect(h.unanswered).toEqual([]);
    const paths = new Set(h.answered.map((a) => a.replace(/^GET /, '')));
    for (const read of ['/_router/health', '/api/projects']) expect(paths, read).toContain(read);
  }, 15_000);

  it("answers the router's health and its projects, shaped as the router sends them", async () => {
    const h = fixtureServer();
    expect(await body(await get(h, '/_router/health'))).toEqual(HEALTH);
    const projects = await body<ProjectRow[]>(await get(h, '/api/projects'));
    expect(projects).toEqual(PROJECTS);
    expect(projects.map((p) => p.project)).toContain(null);
    expect(h.unanswered).toEqual([]);
  });

  it('fails a path no fixture answers loudly: a 501 naming it, the console, and the list', async () => {
    const h = fixtureServer();
    const r = await get(h, '/api/nothing-here');
    expect(r.status).toBe(501);
    expect((await body<{ error: string }>(r)).error).toContain('GET /api/nothing-here');
    expect(h.unanswered).toEqual(['GET /api/nothing-here']);
    expect(errors).toHaveBeenCalledWith(expect.stringContaining('GET /api/nothing-here'));
  });

  it('fails a write, and a query parameter it cannot honour, the same way', async () => {
    const h = fixtureServer();
    expect((await get(h, '/api/projects', { method: 'POST', body: '{}' })).status).toBe(501);
    expect((await get(h, '/api/projects?bogus=1')).status).toBe(501);
    expect(h.unanswered).toEqual(['POST /api/projects', 'GET /api/projects?bogus=1']);
  });

  it('stands for a read that could not be made with a fetch that never got a response', async () => {
    const h = fixtureServer({ unread: ['projects'] });
    await expect(get(h, '/api/projects')).rejects.toThrow(TypeError);
    expect((await get(h, '/_router/health')).status).toBe(200);
    expect(h.unanswered).toEqual([]);
  });

  it('hands anything outside the router to the fetch it replaced', async () => {
    const real = vi.fn(async () => new Response('asset'));
    const h = fixtureServer({ passThrough: real });
    expect(await (await h.fetch(`${BASE}/fonts/IBMPlexSans-Regular.woff2`)).text()).toBe('asset');
    expect(real).toHaveBeenCalledOnce();
    expect(h.answered).toEqual([]);
  });
});
