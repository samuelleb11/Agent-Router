// Copied from Northstar ui/src/components/Refusal.tsx at e9c1589.
// Changed: the titles name the router, and the words come from the router's refusal shapes.
import { InlineNotification } from '@carbon/react';
import { refusalOf, refusalText } from '~/lib/errors';

/// The two states that are not a refusal, each with its own title. Refused, unreachable and
/// reached-with-no-body are three facts, and the last two must not read as each other. A
/// refusal keeps the title the control gives it.
const TITLE = {
  empty: 'Reached the router, but the answer had no body',
  unreachable: 'Could not reach the router',
} as const;

/// A refusal, in the router's own words, beside the control it came from.
///
/// A generic "request failed" throws away the sentence that says what to do. Unreachable and
/// empty are worded apart from refused, because the three call for different responses.
export function Refusal({ error, title = 'Refused' }: { error: unknown; title?: string }) {
  if (!error) return null;
  const r = refusalOf(error);
  const notRefused = r.code === 'unreachable' || r.code === 'empty';
  return (
    <InlineNotification
      kind={notRefused ? 'warning' : 'error'}
      lowContrast
      hideCloseButton
      title={r.code === 'unreachable' ? TITLE.unreachable : r.code === 'empty' ? TITLE.empty : title}
      subtitle={refusalText(error)}
      role="alert"
    />
  );
}
