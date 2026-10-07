// New for agent-router: the copy button beside every snippet in the console. It says when copying
// failed, because the clipboard is refused in some frames and over plain HTTP (ST-C44, ST-C98).
import { useEffect, useId, useRef, useState } from 'react';
import { Button } from '@carbon/react';
import styles from './CopyButton.module.scss';

type Copied = 'idle' | 'copied' | 'failed';

/// Copy a text, and say what happened beside the button: "Copied", or that the clipboard refused
/// and the text has to be selected by hand. Carbon's own copy button falls back to a native
/// `prompt()`, which does nothing in the extension's sandboxed frame (SH-C27), so it is not used.
export function CopyButton({ text, label = 'Copy', what = 'it' }: { text: string; label?: string; what?: string }) {
  const [state, setState] = useState<Copied>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  const statusId = useId();
  const copy = async () => {
    let ok = false;
    try {
      if (!navigator.clipboard?.writeText) throw new Error('no clipboard');
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch {
      ok = false;
    }
    setState(ok ? 'copied' : 'failed');
    if (timer.current) clearTimeout(timer.current);
    // A success fades; a failure stays until the next try, since it says what to do instead.
    if (ok) timer.current = setTimeout(() => setState('idle'), 2500);
  };
  return (
    <span className={styles.copy}>
      <Button kind="tertiary" size="md" onClick={() => void copy()} aria-describedby={state === 'idle' ? undefined : statusId}>
        {label}
      </Button>
      <span id={statusId} className={state === 'failed' ? styles.copyFailed : styles.copyDone} aria-live="polite">
        {state === 'copied' ? 'Copied' : state === 'failed' ? `Copying failed — select ${what} and copy it by hand.` : ''}
      </span>
    </span>
  );
}
