// Copied from Northstar ui/src/app/live.ts at e9c1589.
// Changed: the router has no event stream for the shell to discover, so freshness is its health check, read five seconds after the last read settled: connecting until it first answers, live while it answers, stale once a cycle fails, unreachable after five failed cycles in a row, and reconnecting while an open session's stream is being replaced, which the Sessions screen reports here. Five modes, as Northstar's.
import { useEffect, useState, useSyncExternalStore } from 'react';
import { useQuery } from '@tanstack/react-query';
import { keys } from '~/app/query';
import { readHealth } from '~/data/router';
import { POLL_MS } from '~/lib/freshness';
import type { Phase } from '~/model/session';

/// How the console stands with the router right now, in the header on every page (SH-C10).
///
/// `connecting` is its own state rather than an optimistic "live": before the first answer
/// nothing is known, and a green badge over a router that has not been heard from is a fact
/// invented from an absence. `stale` is one failed refresh, which is a blip until it is not;
/// `unreachable` is five in a row. `reconnecting` is an open session's stream being replaced:
/// what the session last said is no longer known to be true, whatever the health check says.
export type LiveMode = 'connecting' | 'live' | 'stale' | 'unreachable' | 'reconnecting';

/// Five failed refreshes in a row, and the router is called unreachable; fewer read stale. Counted
/// per cycle, and one good cycle sets the count back to zero (SH-C11).
export const UNREACHABLE_AFTER = 5;

/// The health poll's account of itself: failed cycles in a row, and when the last good one finished.
export interface Strikes {
  readonly strikes: number;
  readonly lastOk: number | null;
}

/// The poll's strikes, kept outside React so the read, which runs outside a render, can keep them,
/// and every render reads one value of them.
export function createHealthPoll() {
  let state: Strikes = { strikes: 0, lastOk: null };
  const listeners = new Set<() => void>();
  const put = (next: Strikes) => {
    state = next;
    for (const l of [...listeners]) l();
  };
  return {
    subscribe(l: () => void) {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    snapshot: (): Strikes => state,
    /// One cycle, counted: a good one clears the strikes, a failed one adds one, and one cancelled
    /// by its caller is neither.
    async cycle<T>(read: () => Promise<T>, signal?: AbortSignal): Promise<T> {
      try {
        const value = await read();
        put({ strikes: 0, lastOk: Date.now() });
        return value;
      } catch (error) {
        if (!signal?.aborted) put({ ...state, strikes: state.strikes + 1 });
        throw error;
      }
    },
  };
}

// ---------------------------------------------------------------- the open session's stream

/// What the open session's stream says about itself, while a session is open: the Sessions screen
/// reports it, and the badge reads it. One slot, because one session is open at a time.
let stream: Phase | null = null;
const streamListeners = new Set<() => void>();

function subscribeStream(l: () => void): () => void {
  streamListeners.add(l);
  return () => {
    streamListeners.delete(l);
  };
}

const streamSnapshot = (): Phase | null => stream;

function putStream(next: Phase | null): void {
  if (next === stream) return;
  stream = next;
  for (const l of [...streamListeners]) l();
}

/// Report an open session's stream to the header while the caller is mounted, and nothing once it
/// is gone: closing the session takes its stream out of the badge.
export function useStreamReport(phase: Phase | null): void {
  useEffect(() => {
    putStream(phase);
    return () => putStream(null);
  }, [phase]);
}

// ---------------------------------------------------------------- the badge

/// What the strikes and the stream mean, strongest first: five failed refreshes say the router is
/// gone whatever the stream says; a stream being replaced says the open session's words are not
/// vouched for; one failed refresh says stale; nothing yet says connecting.
export function modeOf(s: Strikes, phase: Phase | null): LiveMode {
  if (s.strikes >= UNREACHABLE_AFTER) return 'unreachable';
  if (phase === 'reconnecting') return 'reconnecting';
  if (s.strikes > 0) return 'stale';
  return s.lastOk === null ? 'connecting' : 'live';
}

/// `GET /_router/health` on the console's beat. It is the one read the router answers to anyone,
/// cheaply, whether or not anything else is working, so it says whether the router is there and
/// nothing more; each screen's own reads say how fresh its figures are.
///
/// The next read starts five seconds after the previous one finished, so reads never overlap or
/// pile up behind a slow router, and nothing is retried quietly: a failed read is a strike, and
/// ~/data/http's timeout makes a hung one fail rather than stop the poll (SH-C13, SH-C15).
export function useLive() {
  const [poll] = useState(createHealthPoll);
  const s = useSyncExternalStore(poll.subscribe, poll.snapshot, poll.snapshot);
  const phase = useSyncExternalStore(subscribeStream, streamSnapshot, streamSnapshot);
  useQuery({
    queryKey: keys.health,
    queryFn: ({ signal }) => poll.cycle(() => readHealth(signal), signal),
    refetchInterval: (query) => (query.state.fetchStatus === 'fetching' ? false : POLL_MS),
    retry: false,
    staleTime: 0,
  });
  const mode = modeOf(s, phase);
  return { mode, label: sentence(mode, s), at: s.lastOk, strikes: s.strikes };
}

const every = `every ${POLL_MS / 1000}s`;

/// A time of day to the second: a poll every five seconds is told apart by its seconds.
const clock = (at: number): string => new Date(at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

/// When the last good read was, or that there never was one: a page that never had an answer must
/// not print a time it does not have.
export const lastGood = (at: number | null): string => (at === null ? 'never read successfully' : `last good read at ${clock(at)}`);

/// The whole sentence, one per mode. The badge shows a word and carries this as its accessible
/// name, so the two never disagree about which state is being described.
export function sentence(mode: LiveMode, s: Strikes): string {
  switch (mode) {
    case 'connecting':
      return `connecting — nothing has answered yet; asking the router ${every}`;
    case 'live':
      return `live — the router answers its health check, asked ${every}`;
    case 'stale':
      return `stale — the last ${s.strikes === 1 ? 'refresh' : `${s.strikes} refreshes`} failed; ${lastGood(s.lastOk)}; asking again ${every}`;
    case 'unreachable':
      return `unreachable — ${s.strikes} refreshes in a row failed; ${lastGood(s.lastOk)}; asking again ${every}`;
    case 'reconnecting':
      return 'reconnecting — the open session’s stream dropped and is being replaced';
  }
}

/// The one word on the badge. Five modes, five words: a word shared between two states is a state
/// the operator cannot see they are in.
export const LIVE_WORD: Readonly<Record<LiveMode, string>> = {
  connecting: 'connecting',
  live: 'live',
  stale: 'stale',
  unreachable: 'unreachable',
  reconnecting: 'reconnecting',
};
