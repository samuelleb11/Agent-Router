// New for agent-router: one settings page's place in the route table, until AGEN-0011 builds it.
import { useParams } from 'react-router';
import { Page } from '~/components/Page';
import { SECTIONS } from '~/lib/routes';
import { Placeholder } from '~/screens/Placeholder';

/// What each page will hold, in the words of the plan (docs/web-console-carbon.md, "Settings").
const WHAT: Readonly<Record<string, string>> = {
  accounts: 'One card per account with Use, Edit and Remove always visible, and Add account in place.',
  'claude-code': 'Start at login, route every project and manage the wrapper, each with what it did.',
  network: 'The switch that puts the router on the LAN, bound to what the router reports, and its address.',
  password: 'The password the LAN gate asks for: set it, change it, remove it with a confirmation.',
  notifications: 'Apple and Firebase push, the registered phones, and test results that stay put.',
  router: 'What this router is: its version, its build and where it keeps its store.',
};

export default function SettingsSection() {
  const { section = '' } = useParams();
  const label = SECTIONS.find((s) => s.slug === section)?.label ?? section;
  return (
    <Page.Section label={label}>
      <Placeholder item="AGEN-0011" what={`The ${label} page`}>
        {WHAT[section] ?? ''}
      </Placeholder>
    </Page.Section>
  );
}
