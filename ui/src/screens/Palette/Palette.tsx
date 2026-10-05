// Copied from Northstar ui/src/screens/Palette/Palette.tsx at e9c1589.
// Changed: the modal only; what it holds is ./Body, loaded the first time the palette opens so that every page's first load does not carry it (scripts/check-chunks.mjs), with a line saying so while it arrives. On a phone the sheet and its overlay both start under the topbar (Palette.module.scss), so the topbar's facts and actions stay in reach while it is open.
import { Suspense, lazy } from 'react';
import { ComposedModal } from '@carbon/react';
import { INPUT_ID } from './ids';
import styles from './Palette.module.scss';

const Body = lazy(() => import('./Body').then((m) => ({ default: m.Body })));

/// The command palette: search the router's conversations, jump to a session or a project, and
/// run commands behind `>`. Not a route: the shell mounts it, and ⌘K or the topbar's search action
/// opens it (SH-C50).
///
/// A passive Carbon modal near the top of the viewport on a desk, where the eye already is, and a
/// full-height sheet in a hand, under the topbar, never over it, so the topbar's own facts and
/// actions stay reachable while the palette is open (SH-C51). The body is mounted only while
/// open, so the typing, the step and the cursor start clean each time and nothing is read while
/// nobody is looking; its code arrives the first time it opens.
export function Palette({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <ComposedModal
      open={open}
      onClose={() => {
        onClose();
      }}
      size="md"
      className={styles.modal}
      containerClassName={styles.container}
      selectorPrimaryFocus={`#${INPUT_ID}`}
      aria-label="Search and commands"
    >
      {open && (
        <Suspense fallback={<Opening />}>
          <Body onClose={onClose} />
        </Suspense>
      )}
    </ComposedModal>
  );
}

/// The palette's frame while its body arrives, the first time it opens: a moment on a router
/// that serves from this machine, and said rather than left blank.
function Opening() {
  return (
    <div className={styles.body} aria-busy="true">
      <p className={styles.opening}>Opening…</p>
    </div>
  );
}
