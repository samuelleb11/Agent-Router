// New for agent-router: where the scoped project's sessions ran — its repository, and every directory on
// the machine that ran it — under the scope bar, at a glance and with nothing to open (OV-C214).
import { useQuery } from '@tanstack/react-query';
import { keys } from '~/app/query';
import type { Snapshot } from '~/data/overview';
import { readProjects } from '~/data/router';
import type { Scope } from '~/model/scope';
import { projectPlaces } from './panels';
import styles from './Overview.module.scss';

export function ProjectPlaces({ scope, snapshot }: { scope: Scope; snapshot: Snapshot | null }) {
  // The rail's read of every project over all time, watched and never asked for here: the shell
  // reads it, so a project quiet in this range still says where it ran. The Projects & sessions tab's,
  // narrowed by the range, stands in when the rail has not read yet.
  const rail = useQuery({ queryKey: keys.projects, queryFn: ({ signal }) => readProjects(signal), enabled: false });
  const places = projectPlaces(scope.project, rail.data, snapshot?.part.tab === 'projects' ? snapshot.part.projects : undefined);
  if (!places) return null;
  const n = places.directories.length;
  return (
    <dl className={styles.places} aria-label="Where this project ran" data-project-places="">
      {places.repository && (
        <div className={styles.place}>
          <dt>Repository</dt>
          <dd className={styles.mono}>{places.repository}</dd>
        </div>
      )}
      {n > 0 && (
        <div className={styles.place}>
          <dt>{n === 1 ? 'Directory' : `${n} directories`}</dt>
          <dd>
            <ul className={styles.placeList}>
              {places.directories.map((d, i) => (
                <li key={`${i}:${d.label}`}>
                  {d.machine && `${d.machine} · `}
                  <span className={styles.mono}>{d.path}</span>
                </li>
              ))}
            </ul>
          </dd>
        </div>
      )}
    </dl>
  );
}
