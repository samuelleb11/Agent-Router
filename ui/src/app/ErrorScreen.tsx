// Copied from Northstar ui/src/app/ErrorScreen.tsx at e9c1589.
// Changed: reworded for the router. A crash here is the console failing, not the router, and the way out is the Overview.
import { Button, Link } from '@carbon/react';
import { Link as RouterLink, isRouteErrorResponse, useLocation, useRouteError } from 'react-router';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import styles from './ErrorScreen.module.scss';

/// What a crash in the console is allowed to say.
///
/// React Router's own screen is a stack trace with a note addressed to the developer, which
/// on an operator's machine is neither usable nor honest about whose fault it is. This one
/// separates the two facts a reader needs and refuses to guess a third:
///
/// 1. **A crash here is the CONSOLE failing, not an answer from the router.** Saying "could
///    not reach the router" would send someone to restart a router that is fine, and every
///    Claude Code session going through it with it.
/// 2. **It says nothing about whether a write landed.** If the page stopped drawing just
///    after an account was switched or a setting saved, the request may well have been sent
///    and answered. A reassuring "nothing was saved" would be a guess, and the expensive kind:
///    a console that invents the outcome of a write is worse than one that admits it does not
///    know. So it says to go and look.
///
/// The error text is shown rather than hidden. It is the one thing that makes a bug
/// reportable, and an operator who cannot see it can only say "it broke".
export function ErrorScreen() {
  const error = useRouteError();
  const location = useLocation();
  const notFound = isRouteErrorResponse(error) && error.status === 404;
  const text = messageOf(error);

  return (
    <Page>
      <PageHeader
        breadcrumb={[{ label: 'Agent Router', to: '/' }, { label: notFound ? 'No such page' : 'Stopped' }]}
        title={notFound ? 'No page at this address' : 'This page stopped working'}
        meta={
          notFound ? (
            <span>
              Nothing is served at <code className={styles.path}>{location.pathname}</code>. It may have been a link
              from somewhere that no longer matches, or a typed address.
            </span>
          ) : (
            <span>
              The console stopped drawing this page. That is a fault in the console, not an answer from the router:
              the router, and every session going through it, may be perfectly well.
            </span>
          )
        }
        action={
          <Button as={RouterLink} to="/">
            Overview
          </Button>
        }
      />

      {!notFound && (
        <Page.Section title="What this does and does not tell you">
          <ul className={styles.facts}>
            <li>
              Nothing the router holds is affected by this. The failure is in drawing the page; no request, session
              or setting is touched by it.
            </li>
            <li>
              <strong>It does not say whether a write landed.</strong> If you had just switched an account or saved a
              setting, the request may have been sent and answered before this happened. Open the page again and read
              what is there rather than assuming either way.
            </li>
            <li>
              Reloading is worth one try: a page can stop working on stale code after the router is updated, and a
              reload fetches the current version.
            </li>
          </ul>
          <div className={styles.actions}>
            <Button kind="tertiary" onClick={() => globalThis.location.reload()}>
              Reload this page
            </Button>
            <Link as={RouterLink} to="/">
              Go to the Overview
            </Link>
          </div>
        </Page.Section>
      )}

      {!notFound && text && (
        <Page.Section title="What failed">
          <p className={styles.lead}>
            The words below are for whoever fixes it. They name the code that stopped, not anything you did.
          </p>
          <pre className={styles.trace}>{text}</pre>
        </Page.Section>
      )}
    </Page>
  );
}

/// Whatever was thrown, as words. A router response carries its own; an Error carries a
/// message and a stack; anything else is shown as it prints, because a thrown string is
/// still the only clue there is.
function messageOf(error: unknown): string {
  if (isRouteErrorResponse(error)) return `${error.status} ${error.statusText}`;
  if (error instanceof Error) return error.stack ?? `${error.name}: ${error.message}`;
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error, null, 2);
  } catch {
    return String(error);
  }
}

/// The same screen for an address no route claims. React Router only throws a 404 for a
/// route it was asked to match, so a catch-all route renders this directly.
export function NotFound() {
  const location = useLocation();
  return (
    <Page>
      <PageHeader
        breadcrumb={[{ label: 'Agent Router', to: '/' }, { label: 'No such page' }]}
        title="No page at this address"
        meta={
          <span>
            Nothing is served at <code className={styles.path}>{location.pathname}</code>. It may have been a link from
            somewhere that no longer matches, or a typed address.
          </span>
        }
        action={
          <Button as={RouterLink} to="/">
            Overview
          </Button>
        }
      />
    </Page>
  );
}
