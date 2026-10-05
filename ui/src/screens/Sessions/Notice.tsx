// New for agent-router: the live view's `say` (assets/live.html), one per place a notice belongs
// rather than one above the composer for every outcome (SE-C178).
import { InlineNotification } from '@carbon/react';
import type { Notice } from '~/model/session';
import styles from './Sessions.module.scss';

/// A notice, framed and dismissible, success and warning told apart, its words as text and never
/// as markup: some carry a reason the router supplied (SE-C177, SE-C178).
export function NoticeLine({ notice, onClose, className }: { notice: Notice | null; onClose: () => void; className?: string }) {
  if (!notice) return null;
  return (
    <InlineNotification
      className={[styles.notice, className].filter(Boolean).join(' ')}
      kind={notice.ok ? 'success' : 'warning'}
      lowContrast
      subtitle={notice.text}
      statusIconDescription={notice.ok ? 'Done' : 'Warning'}
      onClose={() => {
        onClose();
        return false;
      }}
    />
  );
}
