// Copied from Northstar ui/src/app/live.ts at e9c1589.
// Changed: the router has no event stream for the shell to discover, so freshness is its health check, read every 5 s: live while it answers, unreachable once it fails. Three modes, not five.
import { useQuery } from '@tanstack/react-query';
import { keys } from '~/app/query';
import { readHealth } from '~/data/router';
import { POLL_MS } from '~/lib/freshness';

/// How the console stands with the router right now, in the header on every page.
///
/// `unknown` is its own state rather than an optimistic "live": before the first answer
/// nothing is known, and a green badge over a router that has not been heard from is a fact
/// invented from an absence.
export type LiveMode = 'unknown' | 'live' | 'unreachable';

/// `GET /_router/health` on the poll's beat. It is the one read the router answers to anyone,
/// cheaply, whether or not anything else is working, so it says whether the router is there
/// and nothing more; each screen's own reads say how fresh its figures are.
///
/// One failure is enough to say unreachable, and no retry hides it: the next answer turns it
/// back. The interval keeps running through failures, so the badge recovers by itself.
export function useLive() {
  const q = useQuery({
    queryKey: keys.health,
    queryFn: ({ signal }) => readHealth(signal),
    refetchInterval: POLL_MS,
    retry: false,
    staleTime: 0,
  });
  const mode: LiveMode = q.isError ? 'unreachable' : q.data !== undefined ? 'live' : 'unknown';
  return { mode, label: LABEL[mode], at: q.dataUpdatedAt === 0 ? null : q.dataUpdatedAt };
}

const every = `every ${POLL_MS / 1000}s`;

/// The whole sentence, one per mode. The badge shows a word and carries this as its
/// accessible name, so the two never disagree about which state is being described.
const LABEL: Readonly<Record<LiveMode, string>> = {
  unknown: `connecting — asking the router ${every}`,
  live: `live — the router answers its health check, asked ${every}`,
  unreachable: `unreachable — the router did not answer its health check; asking again ${every}`,
};

/// The one word on the badge. Three modes, three words: a word shared between two states is
/// a state the operator cannot see they are in.
export const LIVE_WORD: Readonly<Record<LiveMode, string>> = {
  unknown: 'connecting',
  live: 'live',
  unreachable: 'unreachable',
};
