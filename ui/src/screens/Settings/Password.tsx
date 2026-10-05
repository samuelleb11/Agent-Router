// New for agent-router: Settings → Password (assets/index.html, `pwForm`, `applyAuth`; ST-C73 to
// ST-C79). The heading says whether one is set and when; a deployment's pinned password is said to
// be the deployment's, with nothing to change.
import { Page } from '~/components/Page';
import { useAuth } from '~/data/settings';
import { notes, passwordStatus } from '~/model/settings';
import { Explain, ReadFailed, Reading, Words } from './parts';
import { PasswordForm } from './PasswordForm';
import styles from './Settings.module.scss';

export default function PasswordPage() {
  const auth = useAuth();
  const a = auth.value;
  if (!a) return auth.unreachable ? <ReadFailed what="Could not read whether a password is set" error={auth.error} /> : <Reading what="Reading the password’s state…" />;
  return (
    <Page.Section title="Password">
      <p className={styles.lead}>{passwordStatus(a)}</p>
      <PasswordForm auth={a} />
      <Explain>
        <Words segments={notes(a).password} />
      </Explain>
    </Page.Section>
  );
}
