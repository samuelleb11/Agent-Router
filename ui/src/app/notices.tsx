// Copied from Northstar ui/src/app/notices.tsx at e9c1589.
// Changed: nothing.
import { useSyncExternalStore } from 'react';
import { ToastNotification } from '@carbon/react';
import { dismiss, snapshot, subscribe } from '~/app/notify';
import styles from './notices.module.scss';

/// Where the notices are drawn. The store and `notify` live in `~/app/notify`, so this
/// module exports a component and nothing else.
export function Notices() {
  const list = useSyncExternalStore(subscribe, snapshot);
  if (list.length === 0) return null;
  return (
    <div className={styles.host} aria-live="polite">
      {list.map((n) => (
        <ToastNotification
          key={n.id}
          kind={n.kind}
          title={n.title}
          subtitle={n.detail}
          lowContrast
          onClose={() => {
            dismiss(n.id);
            return false;
          }}
          timeout={0}
        />
      ))}
    </div>
  );
}
