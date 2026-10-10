// New for agent-router: one Settings page, chosen by the address (`/settings/:section`). A page a
// router does not have is not offered, and its address says why rather than drawing a blank or a
// not-found (ST-C101, ST-C103).
import { useEffect, type ComponentType } from 'react';
import { useParams } from 'react-router';
import { InlineNotification } from '@carbon/react';
import { Page } from '~/components/Page';
import { useAuth } from '~/data/settings';
import { SECTIONS, type SectionSlug } from '~/lib/routes';
import { isHosted, settingsPages } from '~/model/settings';
import AccountsPage from './Accounts';
import ClaudeCodePage from './ClaudeCode';
import NetworkPage from './Network';
import NotificationsPage from './Notifications';
import PasswordPage from './Password';
import RouterPage from './Router';
import ForgesPage from './Forges';
import LibraryPage from './Library';
import { NOT_HERE } from '~/model/library';
import { ReadFailed, Reading } from './parts';
import { NO_MACHINE } from './rules';

const PAGES: Readonly<Record<SectionSlug, ComponentType>> = {
  accounts: AccountsPage,
  'claude-code': ClaudeCodePage,
  network: NetworkPage,
  password: PasswordPage,
  notifications: NotificationsPage,
  router: RouterPage,
  forges: ForgesPage,
  library: LibraryPage,
};

const isSection = (s: string): s is SectionSlug => SECTIONS.some((x) => x.slug === s);

export default function SettingsSection() {
  const { section = '' } = useParams();
  const auth = useAuth();
  const label = SECTIONS.find((s) => s.slug === section)?.label ?? section;
  // Each page names itself in the tab's title (SH-C7).
  useEffect(() => {
    document.title = `${label} · Settings · Agent Router`;
  }, [label]);
  // The layout sends an address no page owns to the first page; nothing to draw meanwhile.
  if (!isSection(section)) return null;
  if (!settingsPages(auth.value).includes(section)) {
    if (isHosted(auth.value)) {
      return (
        <Page.Section title={label}>
          <InlineNotification kind="info" lowContrast hideCloseButton title={`${label} is not on this router`} subtitle={NO_MACHINE} />
        </Page.Section>
      );
    }
    // A local router keeps no Library, so its two pages say why (ST-C104).
    if (auth.value !== null) {
      return (
        <Page.Section title={label}>
          <InlineNotification kind="info" lowContrast hideCloseButton title={`${label} is not on this router`} subtitle={NOT_HERE} />
        </Page.Section>
      );
    }
    // Not yet known: offering it first would flash a page a hosted router refuses.
    if (auth.unreachable) return <ReadFailed what="Could not read what kind of router this is" error={auth.error} />;
    return <Reading what="Asking the router what kind it is…" />;
  }
  const Shown = PAGES[section];
  // Keyed by the page, so nothing half-typed on one is carried to another (SH-C54).
  return <Shown key={section} />;
}
