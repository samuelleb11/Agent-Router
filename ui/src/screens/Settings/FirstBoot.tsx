// New for agent-router: the first-boot password prompt (assets/index.html, `#pw`, `applyAuth`;
// ST-C80 to ST-C83). A small modal, loaded only when it opens, which `FirstBootGate` decides.
import { ComposedModal, ModalBody, ModalHeader } from '@carbon/react';
import type { Auth } from '~/model/settings';
import { PasswordForm } from './PasswordForm';
import styles from './Settings.module.scss';

/// Asks once, from this machine, while the network can reach the router and nothing guards it yet.
/// Not now, Escape and setting the password all close it, and each counts as dismissed for the
/// rest of the load (ST-C82).
export default function FirstBoot({ auth, onClose }: { auth: Auth | null; onClose: () => void }) {
  return (
    <ComposedModal
      open
      size="sm"
      aria-label="Set a password"
      preventCloseOnClickOutside
      containerClassName={styles.firstBoot}
      onClose={() => {
        onClose();
        return true;
      }}
    >
      <ModalHeader title="Set a password" label="This router" />
      <ModalBody hasForm>
        <div className={styles.firstBootBody}>
          <p>
            This router is reachable from your network. Other devices will be asked for this password; this Mac never is. Until one is set, other
            devices are refused.
          </p>
          <PasswordForm auth={auth} firstBoot onSet={onClose} onNotNow={onClose} />
        </div>
      </ModalBody>
    </ComposedModal>
  );
}
