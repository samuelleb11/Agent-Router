// New for agent-router: how a machine connects to a hosted router — the installer's command for each
// platform, at the page's own origin, each with its own copy button, and what else the script
// installs where VS Code is (MA-C2). Moved here from the Sessions screen's empty list, which now links
// to the Machines screen instead, so the commands have one home (SE-C187).
import { useId } from 'react';
import { CodeSnippet } from '@carbon/react';
import { CopyButton } from '~/components/CopyButton';
import { CONNECT_HOW, installCommands, WITH_VS_CODE, type InstallCommand } from '~/model/machine';
import styles from './InstallCommands.module.scss';

/// One platform's command, under its label, with its own copy button: a group named by the label,
/// so each Copy says which command it copies.
function Install({ label, command }: InstallCommand) {
  const labelId = useId();
  return (
    <div role="group" aria-labelledby={labelId} className={styles.install}>
      <p id={labelId} className={styles.installLabel}>
        {label}
      </p>
      <div className={styles.installRow}>
        <CodeSnippet type="single" hideCopyButton aria-label={`The command to run on ${label}`}>
          {command}
        </CodeSnippet>
        <CopyButton text={command} label="Copy command" what="the command" />
      </div>
    </div>
  );
}

/// What to run on a machine to connect it to the router at `origin`: the sentence, the command for
/// each platform, and last what else the script installs where VS Code is (MA-C2).
export function InstallCommands({ origin }: { origin: string }) {
  return (
    <div className={styles.connect}>
      <p className={styles.connectHow}>{CONNECT_HOW}</p>
      {installCommands(origin).map((c) => (
        <Install key={c.label} {...c} />
      ))}
      <p className={styles.connectNote}>{WITH_VS_CODE}</p>
    </div>
  );
}
