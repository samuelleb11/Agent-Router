// Copied from Northstar ui/src/data/held.ts at e9c1589.
// Changed: `unwrap` is gone with the generated client it read; reads throw from ~/data/http instead.
import { keepPreviousData, useQueries, useQuery, type UseQueryOptions, type UseQueryResult } from '@tanstack/react-query';
import { held, type Held } from '~/lib/freshness';

/// A query whose result is HELD through a failed refetch and labelled with its age.
///
/// Every read in the console goes through this so the rule lives once: a transient blip
/// leaves the last good value in place with `stale: true` beside it, and the view says
/// "showing what was read 12s ago" rather than replacing the board with an error.
/// `unreachable` is only true when nothing was EVER read — that is the case that renders as
/// "could not reach", never a stale one.
export interface Read<T> extends Held<T> {
  readonly isLoading: boolean;
  readonly unreachable: boolean;
  readonly error: unknown;
}

/// One query result as a view renders it. A single function, so a read made alone and a
/// read made in a fan-out cannot decide `unreachable` differently.
function readOf<T>(q: UseQueryResult<T>): Read<T> {
  const h = held(q.data, q.isError, q.dataUpdatedAt);
  return { ...h, isLoading: q.isLoading, unreachable: q.isError && h.value === null, error: q.error };
}

export function useRead<T>(opts: UseQueryOptions<T>): Read<T> {
  const q = useQuery({ ...opts, placeholderData: (prev) => prev });
  return readOf(q);
}

/// What one element of a fan-out reads: the key the one-at-a-time hook would use, and the
/// fetch it would make.
export interface ReadSpec<T> {
  readonly queryKey: readonly unknown[];
  readonly queryFn: () => Promise<T>;
}

/// Module-level so `useQueries` sees one reference and can keep its combined result stable
/// while nothing underneath changed.
function combineReads<T>(results: UseQueryResult<T>[]): Read<T>[] {
  return results.map((q) => readOf(q));
}

/// One read per element of a list, each held and aged the way `useRead` holds one.
///
/// A hook cannot be called once per element, so this fans out with `useQueries`, keyed
/// EXACTLY as the caller keys — the same `keys.*` and the same fetch the one-at-a-time hook
/// uses — so a page reading one of them and a board reading all of them share one cache
/// entry, and an invalidation moves both. `keepPreviousData` is the "hold what was last read"
/// rule for a fan-out, as `placeholderData: (prev) => prev` is for a single read.
export function useReads<T>(specs: readonly ReadSpec<T>[]): readonly Read<T>[] {
  return useQueries({
    queries: specs.map((s) => ({ queryKey: s.queryKey, queryFn: s.queryFn, placeholderData: keepPreviousData })),
    combine: combineReads<T>,
  });
}

/// Whether a source has been read. An empty list and a list that could not be read are
/// different facts, and a list still on its way is a third; none of them may render as
/// another. Derived from a `Read`: `unreachable` is only true when nothing was ever read, and
/// `at` is null until something was.
export type Availability = 'read' | 'loading' | 'unreachable';

export function availabilityOf(r: { readonly unreachable: boolean; readonly at: number | null }): Availability {
  return r.unreachable ? 'unreachable' : r.at === null ? 'loading' : 'read';
}
