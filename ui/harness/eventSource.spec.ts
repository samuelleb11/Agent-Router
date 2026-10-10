// New for agent-router: the harness's EventSource fake plays its fixtures as the router's stream
// arrives, and fails loudly on a stream it has no fixture for.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeEventSource } from './eventSource';
import type { StreamEvent } from './fixtures';

const STREAMS: Record<string, readonly StreamEvent[]> = {
  s1: [
    { after: 0, data: { role: 'user', kind: 'text', text: 'hello' } },
    { after: 30, event: 'status', data: { tone: 'busy' } },
    { after: 30, data: { role: 'assistant', kind: 'text', text: 'hi' } },
  ],
  gone: [{ after: 0, event: 'error', data: 'no such session' }],
};

describe('fakeEventSource', () => {
  let errors: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    vi.useFakeTimers();
    errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  });
  afterEach(() => {
    vi.useRealTimers();
    errors.mockRestore();
  });

  it('opens, then plays each event in order, by its name, at its time', () => {
    const { EventSource: Fake, answered, unanswered } = fakeEventSource(STREAMS);
    const es = new Fake('/api/live/stream?session=s1');
    const heard: string[] = [];
    es.onopen = () => heard.push('open');
    es.onmessage = (e) => heard.push(`message ${(JSON.parse(e.data as string) as { text: string }).text}`);
    es.addEventListener('status', (e) => heard.push(`status ${(e as MessageEvent).data as string}`));

    expect(es.readyState).toBe(Fake.CONNECTING);
    vi.advanceTimersByTime(0);
    expect(es.readyState).toBe(Fake.OPEN);
    expect(heard).toEqual(['open', 'message hello']);
    vi.advanceTimersByTime(30);
    expect(heard).toEqual(['open', 'message hello', 'status {"tone":"busy"}']);
    vi.advanceTimersByTime(30);
    expect(heard).toEqual(['open', 'message hello', 'status {"tone":"busy"}', 'message hi']);
    expect(answered).toEqual(['STREAM /api/live/stream?session=s1']);
    expect(unanswered).toEqual([]);
  });

  it('delivers a named event only to the listeners registered for its name, as a browser does', () => {
    const { EventSource: Fake } = fakeEventSource(STREAMS);
    const es = new Fake('/api/live/stream?session=s1');
    const messages = vi.fn();
    es.onmessage = messages;
    vi.advanceTimersByTime(100);
    // Two transcript items; the status event reached nobody, as nobody asked for it by name.
    expect(messages).toHaveBeenCalledTimes(2);
  });

  it("sends the router's own error event as a message the error handler hears, and stays open", () => {
    const { EventSource: Fake } = fakeEventSource(STREAMS);
    const es = new Fake('/api/live/stream?session=gone');
    const errorsHeard: Event[] = [];
    es.onerror = (e) => errorsHeard.push(e);
    vi.advanceTimersByTime(10);
    expect(errorsHeard).toHaveLength(1);
    expect((errorsHeard[0] as MessageEvent).data).toBe('no such session');
    expect(es.readyState).toBe(Fake.OPEN);
  });

  it('stops where it is when closed', () => {
    const { EventSource: Fake } = fakeEventSource(STREAMS);
    const es = new Fake('/api/live/stream?session=s1');
    const messages = vi.fn();
    es.onmessage = messages;
    vi.advanceTimersByTime(0);
    es.close();
    vi.advanceTimersByTime(100);
    expect(messages).toHaveBeenCalledTimes(1);
    expect(es.readyState).toBe(Fake.CLOSED);
  });

  it('fails a stream it has no fixture for loudly, CLOSED as a refused stream is, and lists it', () => {
    const { EventSource: Fake, unanswered } = fakeEventSource(STREAMS);
    const es = new Fake('/api/live/stream?session=nobody');
    const failed = vi.fn();
    es.onerror = failed;
    vi.advanceTimersByTime(0);
    expect(failed).toHaveBeenCalledOnce();
    expect(es.readyState).toBe(Fake.CLOSED);
    expect(unanswered).toEqual(['STREAM /api/live/stream?session=nobody']);
    expect(errors).toHaveBeenCalledWith(expect.stringContaining('/api/live/stream?session=nobody'));
    new Fake('/api/live/stream?session=s1&since=4');
    expect(unanswered).toContain('STREAM /api/live/stream?session=s1&since=4');
  });
});
