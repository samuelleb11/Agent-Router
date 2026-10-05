// New for agent-router: Settings → Router. What this router is and how to point a Claude Code at it
// (assets/index.html, `renderRouterInfo`; ST-C95 to ST-C99). The health is read afresh each time the
// page opens, and a failed read is said, never kept as an empty answer for the visit.
import { useQuery } from '@tanstack/react-query';
import { CodeSnippet } from '@carbon/react';
import { keys } from '~/app/query';
import { Page } from '~/components/Page';
import { readHealth } from '~/data/router';
import { useAuth } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { connectSnippet, notes, routerFacts } from '~/model/settings';
import { CopyButton, Explain, Fact, Facts, Words } from './parts';
import styles from './Settings.module.scss';

export default function RouterPage() {
  const auth = useAuth();
  // Its own observer of the shell's health read, which refetches on mount whatever its age.
  const health = useQuery({ queryKey: keys.health, queryFn: ({ signal }) => readHealth(signal), refetchOnMount: 'always', retry: false });
  const read = health.isError && health.fetchStatus === 'idle' ? health.error : null;
  // Until /api/auth answers, the snippet is drawn as for this machine; it is drawn again when the
  // answer changes it (ST-C97).
  const snippet = connectSnippet(location.origin, auth.value);
  return (
    <>
      <Page.Section title="Router">
        <Facts label="This router">
          {routerFacts(health.data, auth.value, location.host).map((f) => (
            <Fact key={f.label} term={f.label} mono={f.label !== 'Version'}>
              {f.value}
            </Fact>
          ))}
        </Facts>
        {read ? (
          <p className={styles.bad} role="alert">
            Could not read the router’s health{health.data ? `; showing what was read ${since(health.dataUpdatedAt)}` : ''}: {refusalText(read)}
          </p>
        ) : null}
        <Explain>
          <Words segments={notes(auth.value).store} />
        </Explain>
      </Page.Section>
      <Page.Section title="Point Claude Code here">
        <Explain>
          Add this to a project’s <code>.claude/settings.local.json</code>:
        </Explain>
        <CodeSnippet type="multi" wrapText hideCopyButton className={styles.snippet} aria-label="What to add to .claude/settings.local.json">
          {snippet}
        </CodeSnippet>
        <div className={styles.actions}>
          <CopyButton text={snippet} what="the snippet" />
        </div>
      </Page.Section>
    </>
  );
}
