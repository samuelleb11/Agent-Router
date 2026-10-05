// New for agent-router: the project list the rail shows, and how a project is named and linked.

import { keys } from '~/app/query';
import { useRead, type Read } from '~/data/held';
import { readProjects, type ProjectRow } from '~/data/router';

/// The key the dashboard has always given a project whose directory was never resolved, so a
/// link from either page scopes to the same thing.
export const UNRESOLVED = '(unresolved)';

/// A project's key in a URL: its working directory, as the router groups by it.
export const projectKey = (row: Pick<ProjectRow, 'project'>): string => row.project ?? UNRESOLVED;

/// A project's name: the last segment of its directory, as the dashboard writes it (`base` in
/// assets/index.html), split on "/" and "\" alike, since a hosted router hears from Windows
/// machines, whose directories are `C:\…` (SE-C222). The full path stays available as the row's
/// own fact.
export function projectName(path: string | null): string {
  if (!path) return UNRESOLVED;
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function useProjects(): Read<ProjectRow[]> {
  return useRead({ queryKey: keys.projects, queryFn: ({ signal }) => readProjects(signal) });
}
