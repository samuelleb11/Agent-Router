// New for agent-router: the Overview's place in the route table, until AGEN-0010 builds it.
import { useSearchParams } from 'react-router';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { projectName } from '~/data/projects';
import { Placeholder } from '~/screens/Placeholder';

/// `/`, and `/?project=<working directory>` from the rail: the Overview, scoped or not. The
/// scope is in the address already, so a link from the rail lands on a page that says which
/// project it is about, built or not.
export default function Overview() {
  const [params] = useSearchParams();
  const project = params.get('project');
  return (
    <Page>
      <PageHeader
        title="Overview"
        meta={
          project === null ? (
            <span>Every project, every session and every account the router serves.</span>
          ) : (
            <span>
              Scoped to {projectName(project)}: <code>{project}</code>
            </span>
          )
        }
      />
      <Page.Section label="Overview">
        <Placeholder item="AGEN-0010" what="The Overview">
          Quota and its burn-down, activity, what the model read, the projects, sessions and requests: the dashboard,
          rebuilt.
        </Placeholder>
      </Page.Section>
    </Page>
  );
}
