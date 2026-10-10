// New for agent-router; Northstar's harness answers fetch alone, and the router's live view
// reads a stream. The EventSource fake the plan asks the harness to add.
import { asked, type Asked } from './asked';
import type { StreamEvent } from './fixtures';

/// The router's one stream: a session's transcript, then what it says about the session.
export const STREAM_PATH = '/api/live/stream';

export interface FixtureStreams extends Asked {
  readonly EventSource: typeof EventSource;
}

/// An `EventSource` that plays fixture events instead of opening a connection, so a screen
/// that reads a stream can be seen and probed with no router.
///
/// It behaves as the browser's does where a screen can tell: CONNECTING until the first tick,
/// then `open`; each fixture event in order, `after` milliseconds after the one before, as an
/// unnamed `message` or as the named event the router sends (`status`, `error`) — and only
/// names a listener registered receive, as only those dispatch in a browser; then the stream
/// stays open and quiet, as a live one does between turns. `close()` stops it where it is.
///
/// **Loud where it cannot answer.** A stream no fixture holds is kept in `unanswered`, logged
/// with `console.error`, and fails the way a refused stream does: an `error` with the source
/// CLOSED, because the browser does not retry a stream that was refused.
export function fakeEventSource(streams: Readonly<Record<string, readonly StreamEvent[]>>, options: { asked?: Asked } = {}): FixtureStreams {
  const { answered, unanswered } = options.asked ?? asked();

  class FixtureEventSource extends EventTarget {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSED = 2;
    readonly CONNECTING = 0;
    readonly OPEN = 1;
    readonly CLOSED = 2;
    readonly url: string;
    readonly withCredentials: boolean;
    readyState = 0;
    onopen: ((this: EventSource, ev: Event) => unknown) | null = null;
    onmessage: ((this: EventSource, ev: MessageEvent) => unknown) | null = null;
    onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
    private timers: ReturnType<typeof setTimeout>[] = [];

    constructor(url: string | URL, init?: EventSourceInit) {
      super();
      const where = new URL(String(url), globalThis.location?.href ?? 'http://harness.invalid/');
      this.url = where.href;
      this.withCredentials = init?.withCredentials ?? false;
      const said = `STREAM ${where.pathname}${where.search}`;
      const keys = [...where.searchParams.keys()];
      const session = where.searchParams.get('session');
      const events = where.pathname === STREAM_PATH && keys.length === 1 && session !== null ? streams[session] : undefined;
      if (!events) {
        unanswered.push(said);
        console.error(`harness: the fixture harness has no stream for ${said}`);
        this.later(0, () => {
          this.readyState = FixtureEventSource.CLOSED;
          this.fire(new Event('error'));
        });
        return;
      }
      answered.push(said);
      this.later(0, () => {
        this.readyState = FixtureEventSource.OPEN;
        this.fire(new Event('open'));
      });
      let at = 0;
      for (const e of events) {
        at += e.after;
        const data = typeof e.data === 'string' ? e.data : JSON.stringify(e.data);
        this.later(at, () => this.fire(new MessageEvent(e.event ?? 'message', { data, origin: where.origin })));
      }
    }

    close(): void {
      this.readyState = FixtureEventSource.CLOSED;
      for (const t of this.timers) clearTimeout(t);
      this.timers = [];
    }

    private later(ms: number, run: () => void): void {
      this.timers.push(setTimeout(run, ms));
    }

    /// To the listeners first, then to the matching `on…` handler, as the browser's own
    /// handlers are listeners of their type.
    private fire(event: Event): void {
      this.dispatchEvent(event);
      const self = this as unknown as EventSource;
      if (event.type === 'open') this.onopen?.call(self, event);
      if (event.type === 'message') this.onmessage?.call(self, event as MessageEvent);
      if (event.type === 'error') this.onerror?.call(self, event);
    }
  }

  return { EventSource: FixtureEventSource as unknown as typeof EventSource, answered, unanswered };
}
