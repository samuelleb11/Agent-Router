// New for agent-router, standing where Northstar's ui/src/screens/Palette/Palette.tsx at e9c1589
// stands: the shell opens it from the header's search action and from ⌘K. AGEN-0007 brings the
// palette itself, with hits from the router's search instead of Northstar's items.
import { Modal } from '@carbon/react';
import styles from './Palette.module.scss';

/// The ⌘K palette's place: it opens, says what it will be, and closes on Escape, the ×, or a
/// click outside. Mounted only while open, so a closed palette leaves nothing in the page.
export function Palette({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null;
  return (
    <Modal open passiveModal size="sm" modalHeading="Search and commands" modalLabel="⌘K" onRequestClose={onClose}>
      <p className={styles.body}>
        The palette is AGEN-0007. It will search the router's conversation index, jump to a project or a session,
        and run commands behind <kbd>&gt;</kbd>: switch account, set a session's model, effort or mode, archive, theme.
      </p>
      <p className={styles.body}>⌘K or Ctrl-K opens it from anywhere, a field included.</p>
    </Modal>
  );
}
