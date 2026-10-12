// New for agent-router: the password form, on the Password page and in the first-boot prompt
// (assets/index.html, `pwForm`; ST-C73 to ST-C83). Its length is counted the way the router counts
// it, in characters, and the password goes exactly as typed, spaces included (ST-C74).
import { useId, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Button, Form, PasswordInput } from '@carbon/react';
import { keys } from '~/app/query';
import { clearPassword, setPassword, useReread } from '~/data/settings';
import { hasAccessTokens, MIN_PASSWORD, passwordForm, passwordProblem, type Auth } from '~/model/settings';
import { Confirm, Outcome } from './parts';
import styles from './Settings.module.scss';

export function PasswordForm({ auth, firstBoot = false, onSet, onNotNow }: { auth: Auth | null; firstBoot?: boolean; onSet?: () => void; onNotNow?: () => void }) {
  const reread = useReread();
  const id = useId();
  const again = useRef<HTMLInputElement>(null);
  const [password, setPw] = useState('');
  const [confirm, setConfirm] = useState('');
  const [tried, setTried] = useState(false);
  const [asking, setAsking] = useState(false);
  const form = passwordForm(auth, firstBoot);

  const set = useMutation({
    mutationFn: async (pw: string) => {
      await setPassword(pw);
      // Set: neither field keeps it, and the router is asked what it now says (ST-C83). A refused
      // one stays in both, to be fixed in place.
      setPw('');
      setConfirm('');
      setTried(false);
      await reread(keys.auth);
    },
    onSuccess: () => onSet?.(),
  });
  const clear = useMutation({
    mutationFn: async () => {
      await clearPassword();
      await reread(keys.auth);
    },
    onSuccess: () => setAsking(false),
  });

  if ('pinned' in form) return <p className={styles.pinned}>{form.pinned}</p>;

  const problem = passwordProblem(password, confirm);
  const tooShort = problem !== null && problem.startsWith('use at least');
  const submit = (e: FormEvent) => {
    e.preventDefault();
    setTried(true);
    set.reset();
    clear.reset();
    if (problem || set.isPending) return;
    set.mutate(password);
  };
  // Enter in the first field goes on to the second rather than sending a password nobody has
  // confirmed; Enter in the second sends it (ST-C73).
  const next = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      again.current?.focus();
    }
  };

  return (
    <Form aria-label={firstBoot ? 'Set a password' : 'The password'} onSubmit={submit} className={styles.form}>
      <div className={styles.fields}>
        <PasswordInput
          id={`${id}-new`}
          labelText="New password"
          autoComplete="new-password"
          spellCheck={false}
          value={password}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={next}
          helperText={`At least ${MIN_PASSWORD} characters.`}
          invalid={tried && tooShort}
          invalidText={tooShort ? `Use at least ${MIN_PASSWORD} characters: this is ${[...password].length}.` : undefined}
        />
        <PasswordInput
          id={`${id}-again`}
          ref={again}
          labelText="Confirm"
          autoComplete="new-password"
          spellCheck={false}
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          helperText="The same again: nobody can see what was typed."
          invalid={tried && problem !== null && !tooShort}
          invalidText={problem !== null && !tooShort ? 'The two entries differ.' : undefined}
        />
      </div>
      <div className={styles.actions}>
        <Button type="submit" kind="primary" size="md" disabled={set.isPending}>
          {form.save}
        </Button>
        {form.secondary === 'Not now' ? (
          <Button kind="secondary" size="md" onClick={onNotNow}>
            Not now
          </Button>
        ) : form.secondary === 'Remove password' ? (
          <Button
            kind="danger--tertiary"
            size="md"
            onClick={() => {
              clear.reset();
              set.reset();
              setAsking(true);
            }}
          >
            Remove password
          </Button>
        ) : null}
      </div>
      <Outcome
        busy={set.isPending}
        busyText="Saving…"
        error={set.error ?? (clear.isPending || asking ? null : clear.error)}
        done={set.isSuccess ? 'Password set.' : clear.isSuccess ? hasAccessTokens(auth) ? 'Password removed. Access tokens remain valid.' : 'Removed — other devices are refused again until a new one is set.' : null}
      />
      <Confirm
        open={asking}
        title="Remove the password?"
        label="Password"
        confirmText="Remove password"
        busy={clear.isPending}
        error={clear.error}
        onClose={() => setAsking(false)}
        onConfirm={() => clear.mutate()}
      >
        <p>{hasAccessTokens(auth) ? 'Devices using this password lose access. Access tokens remain valid.' : 'Every other device is refused until a new one is set from this Mac.'}</p>
        {auth?.from_loopback === false ? (
          <p className={styles.bad}>{hasAccessTokens(auth) ? 'If this browser uses the password, it loses access when the password is removed.' : 'You are reading this from another machine: this browser is locked out the moment it is removed.'}</p>
        ) : null}
      </Confirm>
    </Form>
  );
}
