// Copied from Northstar ui/src/components/LiveBadge.tsx at e9c1589.
// Changed: three modes from the router's health check, not Northstar's five stream modes.
import { Tag } from '@carbon/react';
import { LIVE_WORD, type LiveMode } from '~/app/live';
import styles from './LiveBadge.module.scss';

/// "live" and "unreachable" are different facts about whether anything on this screen can
/// still be current, and the operator is entitled to know which one they are looking at.
///
/// The whole sentence is the badge's ACCESSIBLE NAME rather than a `title`. Carbon's `Tag`
/// takes `title` for the close button of its deprecated filter variant, destructures it out
/// for every other variant and puts its own `title` on the label span, so the sentence
/// would reach no DOM node at any width for any input device.
export function LiveBadge({ mode, label }: { mode: LiveMode; label: string }) {
  const type = mode === 'live' ? 'green' : mode === 'unreachable' ? 'red' : 'gray';
  return (
    <span className={styles.wrap}>
      {/* `role="status"` and not `alert`: the mode changing is worth hearing at the next
          pause, never worth interrupting a sentence for. */}
      <Tag type={type} size="sm" role="status" aria-label={`Freshness: ${label}`} data-live-mode={mode}>
        {LIVE_WORD[mode]}
      </Tag>
    </span>
  );
}
