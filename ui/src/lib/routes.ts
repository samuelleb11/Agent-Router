// Copied from Northstar ui/src/lib/routes.ts at e9c1589.
// Changed: agent-router's three screens and its six settings pages.

/// Where things live: one route table for the rail, the settings sub-navigation and every
/// screen that links to a sibling, so a link and the screen it reaches cannot drift apart. A
/// page is a link, and its address is the state.

/// The pages under `/settings`, in the order they are shown. The slugs are the URL, so a page
/// is a link and the sub-navigation is only a way to reach one.
export const SECTIONS = [
  { slug: 'accounts', label: 'Accounts' },
  { slug: 'claude-code', label: 'Claude Code' },
  { slug: 'network', label: 'Network' },
  { slug: 'password', label: 'Password' },
  { slug: 'notifications', label: 'Notifications' },
  { slug: 'router', label: 'Router' },
] as const;

export type SectionSlug = (typeof SECTIONS)[number]['slug'];

export const settingsPath = (slug: SectionSlug): string => `/settings/${slug}`;

/// The screens, at the paths the old pages answer today: `/` was the dashboard and `/live` the
/// live view, and every bookmark, the VS Code extension, the menubar app and the iPhone open
/// those. A project opens the Overview scoped to it, keyed by its working directory.
export const routes = {
  overview: '/',
  sessions: '/live',
  settings: '/settings',
  project: (key: string) => `/?project=${encodeURIComponent(key)}`,
} as const;
