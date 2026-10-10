// Copied from Northstar ui/src/components/LiveBadge.tsx at e9c1589.
// Changed: the router's five modes — connecting, live, stale, unreachable, reconnecting — each with its own tone; stale wears the console's warning fill, which no Carbon tag has.
import { Tag } from '@carbon/react';
import { LIVE_WORD, type LiveMode } from '~/app/live';
import styles from './LiveBadge.module.scss';

/// Each mode's tone, one per mode: a state still finding out is neutral and never wears the live
/// colour (SH-C10). Carbon's tags have no amber, so stale's tone is the class below.
const TONE: Readonly<Record<LiveMode, 'gray' | 'green' | 'warm-gray' | 'red' | 'magenta'>> = {
  connecting: 'gray',
  live: 'green',
  stale: 'warm-gray',
  unreachable: 'red',
  reconnecting: 'magenta',
};

/// "live" and "unreachable" are different facts about whether anything on this screen can
/// still be current, and the operator is entitled to know which one they are looking at.
///
/// The whole sentence is the badge's ACCESSIBLE NAME rather than a `title`. Carbon's `Tag`
/// takes `title` for the close button of its deprecated filter variant, destructures it out
/// for every other variant and puts its own `title` on the label span, so the sentence
/// would reach no DOM node at any width for any input device.
export function LiveBadge({ mode, label }: { mode: LiveMode; label: string }) {
  return (
    <span className={styles.wrap}>
      {/* `role="status"` and not `alert`: the mode changing is worth hearing at the next
          pause, never worth interrupting a sentence for. */}
      <Tag
        type={TONE[mode]}
        size="sm"
        role="status"
        aria-label={`Freshness: ${label}`}
        data-live-mode={mode}
        className={mode === 'stale' ? styles.stale : undefined}
      >
        {LIVE_WORD[mode]}
      </Tag>
    </span>
  );
}
