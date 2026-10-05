// New for agent-router: what the Sessions specs speak to the screen through. Never imported by the
// screen itself. The fixture harness has its own EventSource fake, which plays a stream on a timer
// (harness/eventSource.ts); a spec needs one it can speak through a word at a time.

/// An EventSource a spec speaks as the router through: a row, a named status, the router's own
/// `error` event, and the connection's failure in either state the browser reports it in. It keeps
/// every instance made, and every event name listened for, since only registered names dispatch.
export class FakeEventSource extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  /// Every connection opened, oldest first.
  static made: FakeEventSource[] = [];

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSED = 2;
  readonly url: string;
  readonly withCredentials = false;
  readyState = 0;
  closed = false;
  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onerror: ((ev: Event) => unknown) | null = null;
  /// The event names something listens for on this connection.
  readonly listened = new Set<string>();

  constructor(url: string | URL) {
    super();
    this.url = String(url);
    FakeEventSource.made.push(this);
  }

  override addEventListener(type: string, listener: EventListenerOrEventListenerObject | null, options?: boolean | AddEventListenerOptions): void {
    this.listened.add(type);
    super.addEventListener(type, listener, options);
  }

  close(): void {
    this.readyState = FakeEventSource.CLOSED;
    this.closed = true;
  }

  /// The connection is up.
  open(): this {
    this.readyState = FakeEventSource.OPEN;
    this.dispatchEvent(new Event('open'));
    return this;
  }

  /// An unnamed event: one feed row.
  row(item: unknown): this {
    this.dispatchEvent(new MessageEvent('message', { data: typeof item === 'string' ? item : JSON.stringify(item) }));
    return this;
  }

  /// A `status` event: what the session is doing.
  status(s: unknown): this {
    this.dispatchEvent(new MessageEvent('status', { data: JSON.stringify(s) }));
    return this;
  }

  /// The router's own `error` event, with its words: "no such session".
  refuse(words = 'no such session'): this {
    this.dispatchEvent(new MessageEvent('error', { data: words }));
    return this;
  }

  /// The connection failing: CONNECTING while the browser retries by itself, CLOSED once it has
  /// given up, as on a refusal.
  drop(state: 0 | 2 = FakeEventSource.CONNECTING): this {
    this.readyState = state;
    this.dispatchEvent(new Event('error'));
    return this;
  }

  /// The connections still open.
  static live(): FakeEventSource[] {
    return FakeEventSource.made.filter((s) => !s.closed);
  }

  /// The newest connection.
  static last(): FakeEventSource {
    const s = FakeEventSource.made[FakeEventSource.made.length - 1];
    if (!s) throw new Error('no EventSource was opened');
    return s;
  }

  static reset(): void {
    FakeEventSource.made = [];
  }
}

/// The fake as the type the hook and the screen take.
export const fakeSource = FakeEventSource as unknown as typeof EventSource;
