// New for agent-router: the Sessions screen's place in the route table, until AGEN-0012 builds it.
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { Placeholder } from '~/screens/Placeholder';

/// `/live`: the live view's address, kept, because the VS Code extension, the menubar app and
/// every bookmark open it.
export default function Sessions() {
  return (
    <Page>
      <PageHeader title="Sessions" meta={<span>Every Claude Code session going through the router, live.</span>} />
      <Page.Section label="Sessions">
        <Placeholder item="AGEN-0012" what="Sessions">
          The session list beside its feed, the prompts that wait on you, and the composer: the live view, rebuilt.
        </Placeholder>
      </Page.Section>
    </Page>
  );
}
