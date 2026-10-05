// New for agent-router: one session's stream, as the live view's `connect` and its watchdog kept it
// (assets/live.html), on the session model's rules (~/model/session). The hook owns the
// EventSource and the timers; every decision — what a row, a status or an error does to the feed,
// when silence means a dead link — is the model's, so the phones' parity tests and this hook read
// the same rules.
//
// Where Northstar's EventSource hooks went wrong, and the page too, the rules are kept:
// - a replaced connection is history, whatever it still delivers (SE-C56): each connection carries
//   its number, and only the newest one's events change anything;
// - an error while CONNECTING is the browser reconnecting (Northstar 1374e53): the header stops
//   vouching at once (SE-C64, SE-C65);
// - only registered names dispatch (Northstar bcd16e2): every event the router sends is listened
//   for by name (SE-C58);
// - the router's own `error` event, and a caller it refuses, are said and never retried (SE-C59,
//   SE-C60); the page took both for a dropped link and retried them every ten seconds for good;
// - React StrictMode mounts, unmounts and mounts again: the stream opens a tick after the mount
//   that keeps it, so the double mount opens one stream, not two.

import { useCallback, useEffect, useRef, useState } from 'react';
import { streamRefusal, streamUrl } from '~/data/live';
import { feedAside, feedEcho, feedError, feedHeader, feedMessage, feedRefused, feedStatus, feedTick, openFeed, type Feed, type Pip } from '~/model/session';

/// Every event the router sends on a session's stream (src/live.rs, `stream`): an unnamed one is a
/// feed row (`message`), `status` is what the session is doing, and `error` is either the router
/// refusing the stream, with its words, or the connection's own failure, with none (SE-C58).
export const STREAM_EVENTS = ['message', 'status', 'error'] as const;

/// How often the watchdog looks for ten seconds of silence (SE-C66, SE-C211).
export const WATCH_MS = 1000;

/// `EventSource.CLOSED`: the browser has given the connection up and will not retry it.
const CLOSED = 2;

export interface StreamOptions {
  /// The EventSource to open; the browser's unless a spec passes a fake.
  readonly EventSource?: typeof EventSource;
  /// The clock the silence is measured on; this machine's, which is all a silence can be told by.
  readonly now?: () => number;
  /// Asked once the browser has given a stream up: the router's words when it refuses the caller
  /// (a 401), else null, and the watchdog reconnects (SE-C60, SE-C65).
  readonly refusal?: (session: string, signal: AbortSignal) => Promise<string | null>;
}

export interface SessionStream {
  /// The open session's stream and feed, or null with no session open. Never another session's:
  /// the feed is held with the id it was opened for, and read back only for that id (SE-C193).
  readonly feed: Feed | null;
  /// The header the stream earns: "connecting…", the status, "reconnecting…" or the refusal.
  readonly header: Pip | null;
  /// A message sent, echoed at once (SE-C150), or a side question, with its label (SE-C160).
  readonly echo: (text: string, label?: string) => void;
  /// A side question's answer, landed in the feed (SE-C160).
  readonly aside: (answer: string | null | undefined) => void;
}

interface Held {
  readonly session: string;
  readonly feed: Feed;
}

/// A connection's own failure carries no data; the router's `error` event carries its words.
const fromRouter = (e: Event): e is MessageEvent<string> => typeof (e as { data?: unknown }).data === 'string';

export function useSessionStream(session: string | null, options: StreamOptions = {}): SessionStream {
  const [held, setHeld] = useState<Held | null>(null);
  const ref = useRef<Held | null>(null);
  // The latest options, read when they are used, so a caller passing a fresh object each render
  // does not reconnect the stream.
  const opts = useRef(options);
  useEffect(() => {
    opts.current = options;
  });

  /// Change the feed of `owner`, or nothing when another session's is held by now.
  const update = useCallback((owner: string, change: (f: Feed) => Feed) => {
    const h = ref.current;
    if (!h || h.session !== owner) return;
    const next = change(h.feed);
    if (next === h.feed) return;
    ref.current = { session: owner, feed: next };
    setHeld(ref.current);
  }, []);

  useEffect(() => {
    if (session === null) {
      ref.current = null;
      setHeld(null);
      return;
    }
    const now = () => (opts.current.now ?? Date.now)();
    const Source = opts.current.EventSource ?? globalThis.EventSource;
    const asking = opts.current.refusal ?? streamRefusal;
    let disposed = false;
    let source: EventSource | null = null;
    let probe: AbortController | null = null;

    ref.current = { session, feed: openFeed(now()) };
    setHeld(ref.current);

    /// Listen to connection number `connection`, closing whatever was listened to before.
    const open = (connection: number) => {
      source?.close();
      probe?.abort();
      probe = null;
      const es = new Source(streamUrl(session));
      source = es;
      const current = () => !disposed && source === es;
      es.addEventListener('message', (e) => {
        if (current()) update(session, (f) => feedMessage(f, connection, String((e as MessageEvent).data), now()));
      });
      es.addEventListener('status', (e) => {
        if (current()) update(session, (f) => feedStatus(f, connection, String((e as MessageEvent).data), now()));
      });
      es.addEventListener('error', (e) => {
        if (!current()) return;
        if (fromRouter(e)) {
          // The router speaking: "no such session". Closed here, or the browser would retry it.
          es.close();
          update(session, (f) => feedRefused(f, connection, e.data));
          return;
        }
        // CONNECTING: the browser is retrying by itself. CLOSED: it gave up, and the watchdog
        // reconnects in its own time — unless the router refuses the caller, which is asked.
        update(session, (f) => feedError(f, connection));
        if (es.readyState !== CLOSED) return;
        const asked = new AbortController();
        probe = asked;
        asking(session, asked.signal).then(
          (words) => {
            if (words === null || asked.signal.aborted || !current()) return;
            es.close();
            update(session, (f) => feedRefused(f, connection, words));
          },
          () => {},
        );
      });
    };

    /// Ten seconds of nothing is a link that died without a word: a new connection replaces it.
    const tick = () => {
      const h = ref.current;
      if (disposed || !h || h.session !== session) return;
      const { feed, reconnect } = feedTick(h.feed, now());
      update(session, () => feed);
      if (reconnect) open(feed.connection);
    };

    // A tick after the mount that keeps it: StrictMode's first mount is undone before this runs.
    const start = setTimeout(() => {
      if (!disposed) open(ref.current?.feed.connection ?? 1);
    }, 0);
    const watchdog = setInterval(tick, WATCH_MS);
    // A page shown again looks at once: a phone backgrounds it for minutes, and its timers with
    // it, so a stream that went quiet meanwhile is replaced now, not a second from now (SE-C74).
    const onVisible = () => {
      if (document.visibilityState === 'visible') tick();
    };
    document.addEventListener('visibilitychange', onVisible);

    return () => {
      // Leaving closes the connection at once, not when a timer would (SE-C57).
      disposed = true;
      clearTimeout(start);
      clearInterval(watchdog);
      document.removeEventListener('visibilitychange', onVisible);
      probe?.abort();
      source?.close();
      source = null;
    };
  }, [session, update]);

  const echo = useCallback(
    (text: string, label?: string) => {
      if (session !== null) update(session, (f) => feedEcho(f, text, label));
    },
    [session, update],
  );
  const aside = useCallback(
    (answer: string | null | undefined) => {
      if (session !== null) update(session, (f) => feedAside(f, answer));
    },
    [session, update],
  );

  const feed = held && held.session === session ? held.feed : null;
  return { feed, header: feed ? feedHeader(feed) : null, echo, aside };
}
