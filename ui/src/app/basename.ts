// New for agent-router: where the console is mounted.

/// The console is served at /next/ until it replaces the old pages (docs/web-console-carbon.md,
/// Decision J), and at / after. One build serves both, so the router's basename is read from the
/// address the page was opened at rather than built in: "/next" when the path is /next or lies
/// under it, and "" otherwise. A whole segment, so /nextcloud is not the console at /next.
export function basenameFor(pathname: string): string {
  return pathname === '/next' || pathname.startsWith('/next/') ? '/next' : '';
}
