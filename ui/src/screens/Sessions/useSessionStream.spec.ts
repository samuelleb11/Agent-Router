// New for agent-router: the stream hook against a fake EventSource the spec speaks as the router
// through, on a clock the spec moves (SE-C211): ten seconds of silence are ten seconds of fake time.
// The router's clock is seven minutes behind the spec's, so a duration measured on the wrong one
// would show (SE-C212).
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { StrictMode, createElement, type ReactNode } from 'react';
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Feed } from '~/model/session';
import { FakeEventSource, fakeSource } from './fakes';
import { STREAM_EVENTS, useSessionStream, type StreamOptions } from './useSessionStream';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);
const AT = NOW - 7 * 60e3;

const status = (activity: string, extra: object = {}) => ({
  state: 'working',
  detail: activity === 'tool' ? 'Running Bash' : 'Thinking',
  activity,
  tone: 'busy',
  as_of_ms: AT,
  since_ms: AT - 12e3,
  quiet_ms: 12e3,
  queued: 0,
  ...extra,
});
const row = (text: string) => ({ role: 'assistant', kind: 'text', text });
const said = (feed: Feed | null): string[] =>
  (feed?.entries ?? []).map((e) => (e.kind === 'row' ? String(e.item.text) : `${e.label}: ${e.text}`));

const never = async () => null;

function mount(id: string | null = 'aaa-1', options: StreamOptions = {}, wrapper?: (p: { children: ReactNode }) => ReactNode) {
  const hook = renderHook(({ id: session }: { id: string | null }) => useSessionStream(session, { EventSource: fakeSource, refusal: never, ...options }), {
    initialProps: { id },
    ...(wrapper ? { wrapper } : {}),
  });
  // The stream opens a tick after the mount that keeps it.
  act(() => {
    vi.advanceTimersByTime(0);
  });
  return hook;
}

const wait = (ms: number) =>
  act(() => {
    vi.advanceTimersByTime(ms);
  });

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  FakeEventSource.reset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useSessionStream', () => {
  it('SE-C58: every named event the router sends is registered — rows, status and error — and no other', () => {
    mount();
    const rust = readFileSync(join(__dirname, '../../../../src/live.rs'), 'utf8');
    // What the router names in its stream, read from its source rather than remembered here.
    const named = [...rust.matchAll(/Event::default\(\)\s*\.event\("([a-z_]+)"\)/g)].map((m) => m[1]);
    expect(new Set(named)).toEqual(new Set(['status', 'error']));
    for (const name of [...named, 'message']) expect(STREAM_EVENTS).toContain(name);
    expect([...FakeEventSource.last().listened].sort()).toEqual([...STREAM_EVENTS].sort());
  });

  it('SE-C55: opening a session connects to its stream by its real id, URL-encoded, and with no session nothing is watched', () => {
    const { rerender } = mount(null);
    expect(FakeEventSource.made).toHaveLength(0);
    rerender({ id: 'a b/c?d' });
    wait(0);
    expect(FakeEventSource.last().url).toBe('/api/live/stream?session=a%20b%2Fc%3Fd');
  });

  it('SE-C69, SE-C70: a replay replaces the feed only on a fresh connection’s first event, never on the first connection’s', () => {
    const { result } = mount();
    const es = FakeEventSource.last().open();
    // Echoed before the first connection spoke: it stays, since nothing has been read twice yet.
    act(() => result.current.echo('sent at once'));
    act(() => {
      es.row(row('one')).row(row('two')).status(status('generating'));
    });
    expect(said(result.current.feed)).toEqual(['you · queued: sent at once', 'one', 'two']);
    // The link drops: what was read stays on screen while the browser retries by itself.
    act(() => {
      es.drop(FakeEventSource.CONNECTING);
    });
    expect(said(result.current.feed)).toEqual(['you · queued: sent at once', 'one', 'two']);
    // The retry replays the tail: its first event starts a new copy, and the tail lands once.
    act(() => {
      es.open().row(row('one'));
    });
    expect(said(result.current.feed)).toEqual(['one']);
    act(() => {
      es.row(row('two')).row(row('three'));
    });
    expect(said(result.current.feed)).toEqual(['one', 'two', 'three']);
  });

  it('SE-C64, SE-C65: an error while CONNECTING reads as reconnecting — Stop goes — and the next status is believed again', () => {
    const { result } = mount();
    const es = FakeEventSource.last().open();
    expect(result.current.header).toEqual({ tone: 'stale', text: 'connecting…', stop: false });
    act(() => {
      es.status(status('tool'));
    });
    expect(result.current.header).toEqual({ tone: 'busy', text: 'Running Bash · 12s', stop: true });
    act(() => {
      es.drop(FakeEventSource.CONNECTING);
    });
    expect(result.current.header).toEqual({ tone: 'stale', text: 'reconnecting…', stop: false });
    // The browser retries on its own: no second connection is opened for it.
    expect(FakeEventSource.made).toHaveLength(1);
    act(() => {
      es.open().status(status('tool'));
    });
    expect(result.current.header?.stop).toBe(true);
  });

  it('SE-C66, SE-C67: ten seconds of silence gives the stream up and reconnects, and each connection gets ten seconds of its own', () => {
    const { result } = mount();
    const first = FakeEventSource.last().open();
    act(() => {
      first.row(row('kept')).status(status('tool'));
    });
    wait(9000);
    // Every status starts the ten seconds again.
    act(() => {
      first.status(status('tool'));
    });
    wait(9000);
    expect([FakeEventSource.made.length, result.current.header?.stop]).toEqual([1, true]);
    wait(1000);
    // Silent for ten seconds: closed, replaced, and the header no longer vouches.
    expect(first.closed).toBe(true);
    expect(FakeEventSource.made).toHaveLength(2);
    expect(FakeEventSource.live()).toHaveLength(1);
    expect(result.current.header).toEqual({ tone: 'stale', text: 'reconnecting…', stop: false });
    // What was read is still there to read.
    expect(said(result.current.feed)).toEqual(['kept']);
    // A connection that never says anything is retried every ten seconds, not every second.
    wait(9000);
    expect(FakeEventSource.made).toHaveLength(2);
    wait(1000);
    expect(FakeEventSource.made).toHaveLength(3);
    expect(FakeEventSource.live()).toEqual([FakeEventSource.last()]);
  });

  it('SE-C56: events from a replaced connection are ignored — a row, a status, an error, a refusal', () => {
    const { result } = mount();
    const first = FakeEventSource.last().open();
    act(() => {
      first.row(row('one')).status(status('tool'));
    });
    wait(10_000);
    const second = FakeEventSource.last();
    expect(second).not.toBe(first);
    const before = result.current.feed;
    act(() => {
      first.row(row('late')).status(status('generating')).drop(FakeEventSource.CLOSED).refuse('no such session');
    });
    expect(result.current.feed).toBe(before);
    expect(result.current.header?.text).toBe('reconnecting…');
    // The connection that replaced it is believed, and its replay starts a new copy.
    act(() => {
      second.open().row(row('one')).status(status('generating'));
    });
    expect(said(result.current.feed)).toEqual(['one']);
    expect(result.current.header?.text).toBe('Thinking · 12s');
  });

  it('SE-C59: the router’s own error event is said in its words and never retried', () => {
    const { result } = mount();
    const es = FakeEventSource.last().open();
    act(() => {
      es.refuse('no such session');
    });
    expect(result.current.header).toEqual({ tone: 'stale', text: 'no such session', stop: false });
    // Closed at once, so the browser does not retry it, and the watchdog never does.
    expect(es.closed).toBe(true);
    wait(60_000);
    expect(FakeEventSource.made).toHaveLength(1);
  });

  it('SE-C60, SE-C65: once the browser gives a stream up, a caller the router refuses is told so and never retried', async () => {
    const asked: string[] = [];
    const refusal = async (session: string) => {
      asked.push(session);
      return 'agent-router: this router needs its password';
    };
    const { result } = mount('aaa-1', { refusal });
    const es = FakeEventSource.last();
    act(() => {
      es.drop(FakeEventSource.CLOSED);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(asked).toEqual(['aaa-1']);
    expect(result.current.header).toEqual({ tone: 'stale', text: 'agent-router: this router needs its password', stop: false });
    wait(60_000);
    expect(FakeEventSource.made).toHaveLength(1);
  });

  it('SE-C65: a stream the browser gave up on, from a router that does not refuse the caller, is reconnected by the watchdog', async () => {
    const { result } = mount();
    const es = FakeEventSource.last().open();
    act(() => {
      es.status(status('tool')).drop(FakeEventSource.CLOSED);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.header?.text).toBe('reconnecting…');
    expect(FakeEventSource.made).toHaveLength(1);
    wait(10_000);
    expect(FakeEventSource.made).toHaveLength(2);
  });

  it('SE-C68: a clock set back never holds the next reconnect off by however far it moved', () => {
    mount();
    const es = FakeEventSource.last().open();
    act(() => {
      es.status(status('tool'));
    });
    // Set back an hour: the silence is measured from no later than now, so the watchdog's next
    // look starts it there, and ten seconds after that the link is replaced — not an hour after.
    vi.setSystemTime(NOW - 3600e3);
    wait(9000);
    expect(FakeEventSource.made).toHaveLength(1);
    wait(2000);
    expect(FakeEventSource.made).toHaveLength(2);
  });

  it('SE-C74: a page shown again reconnects at once when the stream went quiet while it was hidden', () => {
    mount();
    const es = FakeEventSource.last().open();
    act(() => {
      es.status(status('tool'));
    });
    // Hidden: a phone stops the page's timers, while the clock goes on.
    vi.setSystemTime(NOW + 45_000);
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(FakeEventSource.made).toHaveLength(2);
    expect(es.closed).toBe(true);
  });

  it('SE-C57, SE-C194: leaving closes the connection at once, and another session starts its own feed from nothing', () => {
    const { result, rerender, unmount } = mount('aaa-1');
    const a = FakeEventSource.last().open();
    act(() => {
      a.row(row('from a')).status(status('tool'));
    });
    rerender({ id: 'bbb-2' });
    expect(a.closed).toBe(true);
    // Another session's rows go at once, before its own stream has said anything.
    expect(result.current.feed?.entries).toEqual([]);
    expect(result.current.header?.text).toBe('connecting…');
    wait(0);
    expect(FakeEventSource.last().url).toBe('/api/live/stream?session=bbb-2');
    unmount();
    expect(FakeEventSource.live()).toEqual([]);
  });

  it('SE-C193: an echo or a side answer for a session no longer on screen lands nowhere', () => {
    const { result, rerender } = mount('aaa-1');
    const { echo, aside } = result.current;
    rerender({ id: 'bbb-2' });
    wait(0);
    act(() => {
      echo('typed in a');
      aside('answered for a');
    });
    expect(result.current.feed?.entries).toEqual([]);
  });

  it('SE-C56: a StrictMode double mount opens one stream, and it is the one listened to', () => {
    const strict = ({ children }: { children: ReactNode }) => createElement(StrictMode, null, children);
    const { result } = mount('aaa-1', {}, strict);
    expect(FakeEventSource.made).toHaveLength(1);
    const es = FakeEventSource.last().open();
    act(() => {
      es.row(row('heard')).status(status('generating'));
    });
    expect(said(result.current.feed)).toEqual(['heard']);
    expect(result.current.header?.text).toBe('Thinking · 12s');
  });
});
