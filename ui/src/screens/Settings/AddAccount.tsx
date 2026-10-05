// New for agent-router: adding an account, in place under the Accounts heading (assets/index.html,
// `wireAddForm`; ST-C39 to ST-C52). Each way to add one is a choice that carries its explanation;
// the sign-in link and the profile command sit in code snippets with a copy button that says when
// copying failed; nothing opens a window by itself.
import { useEffect, useId, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { Button, CodeSnippet, Form, InlineLoading, PasswordInput, RadioTile, TextInput, Tile, TileGroup } from '@carbon/react';
import { keys } from '~/app/query';
import { addAccount, probeProfile, readLogin, startSignin, useReread, type AccountRow } from '~/data/settings';
import { accountSources, addForm, isHosted, notes, type Auth, type Source } from '~/model/settings';
import { accountId, looksLikeEmail, nameProblem, PROFILE_POLL_MS, running, signin, SIGNIN_POLL_MS, signinWords, TOKEN_NEEDED, tokenProblem, type Signin } from './rules';
import { CopyButton, Explain, Outcome, ReadFailed, Words } from './parts';
import styles from './Settings.module.scss';

/// Each way in, as its choice says it (ST-C40, ST-C43).
const CHOICE: Readonly<Record<Source, { readonly title: string; readonly body: readonly (string | { readonly code: string } | { readonly b: string })[] }>> = {
  signin: {
    title: 'Sign in from here (recommended)',
    body: [
      'The router runs ',
      { code: 'claude setup-token' },
      ' for you and issues a ',
      { b: 'long-lived (one-year)' },
      ' credential, so it never needs refreshing. Sign in as the account you want to add — not the one you are already using.',
    ],
  },
  token: {
    title: 'Paste a token I already have',
    body: ['The value ', { code: 'claude setup-token' }, ' prints, ', { code: 'sk-ant-oat01-…' }, ', signed in as the account to add.'],
  },
  profile: {
    title: 'Separate Claude Code profile',
    body: [addForm('profile').help ?? ''],
  },
};

function Body({ parts }: { parts: (typeof CHOICE)[Source]['body'] }) {
  return (
    <>
      {parts.map((p, i) => (typeof p === 'string' ? <span key={i}>{p}</span> : 'code' in p ? <code key={i}>{p.code}</code> : <b key={i}>{p.b}</b>))}
    </>
  );
}

/// The sign-in flow the router drives: its status read about every 1.5 s while it runs, one failed
/// read let pass, and the reading stopped when the flow ends (ST-C45). A flow already running when
/// the form opens is picked up at once (ST-C46). Done and failed are reported only for a flow this
/// form saw running, so an old flow's ending is not news.
function useSignin(enabled: boolean) {
  const [status, setStatus] = useState<Signin>({ stage: 'none' });
  const [seen, setSeen] = useState(false);
  const [misses, setMisses] = useState<{ readonly count: number; readonly error: unknown }>({ count: 0, error: null });
  const [round, setRound] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const step = async () => {
      try {
        const now = signin(await readLogin());
        if (stopped) return;
        setMisses({ count: 0, error: null });
        setStatus(now);
        if (!running(now)) return;
        setSeen(true);
      } catch (error) {
        if (stopped) return;
        setMisses((m) => ({ count: m.count + 1, error }));
      }
      timer = setTimeout(() => void step(), SIGNIN_POLL_MS);
    };
    void step();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  }, [enabled, round]);
  return {
    status,
    seen,
    misses,
    /// A flow this form started: show what the router answered, and read on from there.
    begin: (s: Signin) => {
      setStatus(s);
      setSeen(true);
      setRound((r) => r + 1);
    },
  };
}

/// A new profile's login, checked every 3 s until it reports logged in (ST-C51). A failed check is
/// let pass: the login is a terminal program the reader is still in.
function useProfileWatch(id: string | null, onIn: (who: string) => void) {
  const latest = useRef(onIn);
  useEffect(() => {
    latest.current = onIn;
  });
  useEffect(() => {
    if (!id) return;
    let stopped = false;
    const timer = setInterval(() => {
      void probeProfile(id).then(
        (st) => {
          if (!stopped && st.loggedIn) {
            stopped = true;
            clearInterval(timer);
            latest.current(st.email || id);
          }
        },
        () => {},
      );
    }, PROFILE_POLL_MS);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [id]);
}

export function AddAccount({ auth, accounts, onDone, onClose }: { auth: Auth | null; accounts: readonly AccountRow[]; onDone: (said: string) => void; onClose: () => void }) {
  const reread = useReread();
  const sources = accountSources(auth);
  const offered = sources.join(',');
  const [picked, setPicked] = useState<Source>(sources[0] ?? 'token');
  // A choice the router no longer offers (it said it is hosted) gives way to the first one left.
  const source: Source = sources.includes(picked) ? picked : (sources[0] ?? 'token');
  const [label, setLabel] = useState('');
  const [token, setToken] = useState('');
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [profile, setProfile] = useState<{ readonly id: string; readonly command: string } | null>(null);
  const flow = useSignin(source === 'signin');
  const groupName = useId();
  const form = addForm(source);

  const nameIssue = nameProblem(label, accounts);
  // The name is checked as it is typed once it says something; empty, only once Add was pressed.
  const showName = (label.trim() !== '' || tried) && !!nameIssue;
  const tokenIssue = source === 'token' ? (token.trim() === '' ? TOKEN_NEEDED : tokenProblem(token)) : null;
  const id = accountId(label);

  const finish = (said: string) => {
    void reread(keys.accounts);
    onDone(said);
  };
  useProfileWatch(profile?.id ?? null, (who) => finish(`Signed in as ${who}.`));
  // A sign-in this form saw running has ended signed in: the account is there to read.
  const ended = flow.seen && flow.status.stage === 'done' ? signinWords(flow.status) : null;
  const finishing = useRef(finish);
  useEffect(() => {
    finishing.current = finish;
  });
  useEffect(() => {
    if (ended) finishing.current(ended);
  }, [ended]);

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    setTried(true);
    if (busy || nameIssue || tokenIssue) return;
    setBusy(true);
    setError(null);
    try {
      if (source === 'signin') {
        flow.begin(signin(await startSignin(label)));
      } else if (source === 'token') {
        await addAccount(label, 'token', token);
        // Stored: the secret leaves the screen. On a refusal it stays where it was pasted (ST-C50).
        setToken('');
        finish(`Added ${label.trim()}.`);
      } else {
        const added = await addAccount(label, 'profile');
        void reread(keys.accounts);
        if (added.command) setProfile({ id: added.id, command: added.command });
        else finish(`Added ${label.trim()}.`);
      }
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  const escape = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && !busy) {
      e.stopPropagation();
      onClose();
    }
  };

  const signing = source === 'signin' && flow.seen && running(flow.status);
  const url = flow.status.stage === 'waiting' || flow.status.stage === 'starting' ? flow.status.url : null;

  return (
    <Tile className={styles.addForm}>
      <Form aria-label="Add an account" onSubmit={(e) => void submit(e)} onKeyDown={escape} className={styles.form}>
        <h3 className={styles.cardTitle}>Add an account</h3>
        {sources.length > 1 ? (
          <TileGroup key={offered} legend="How to add it" name={groupName} valueSelected={source} onChange={(v) => setPicked(v as Source)} className={styles.choices}>
            {sources.map((s) => (
              <RadioTile key={s} id={`${groupName}-way-${s}`} value={s} className={styles.choice}>
                <span className={styles.choiceTitle}>{CHOICE[s].title}</span>
                <span className={styles.choiceBody}>
                  <Body parts={CHOICE[s].body} />
                </span>
              </RadioTile>
            ))}
          </TileGroup>
        ) : (
          <Explain>
            Paste the long-lived token that <code>claude setup-token</code> prints{isHosted(auth) ? ': a hosted router adds an account no other way.' : '.'}
          </Explain>
        )}

        <TextInput
          id={`${groupName}-name-field`}
          labelText="Name"
          placeholder="work"
          autoComplete="off"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          invalid={showName}
          invalidText={nameIssue ?? undefined}
          helperText={
            source === 'token' && looksLikeEmail(label)
              ? 'This name is an email address, so it becomes the account’s identity: a long-lived token cannot report which account it belongs to.'
              : id
                ? `Its id will be “${id}”.`
                : 'What the account is called here.'
          }
          disabled={signing}
        />

        {source === 'token' ? (
          <PasswordInput
            id={`${groupName}-token-field`}
            labelText="Token"
            placeholder="sk-ant-oat01-…"
            autoComplete="new-password"
            spellCheck={false}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            invalid={tried && !!tokenIssue}
            invalidText={tokenIssue ?? undefined}
            helperText={<Words segments={notes(auth).token} />}
          />
        ) : null}

        {source === 'signin' && flow.seen ? <SigninPanel status={flow.status} url={url} misses={flow.misses} /> : null}

        {profile ? (
          <div className={styles.flow}>
            <Explain>Run this in a terminal — this page will notice when you are done:</Explain>
            <CodeSnippet type="multi" wrapText hideCopyButton className={styles.snippet} aria-label="The command to run">
              {profile.command}
            </CodeSnippet>
            <div className={styles.actions}>
              <CopyButton text={profile.command} label="Copy command" what="the command" />
            </div>
            <InlineLoading description="Waiting for the profile’s login…" status="active" />
          </div>
        ) : null}

        <div className={styles.actions}>
          {!signing && !profile ? (
            <Button type="submit" kind="primary" size="md" disabled={busy}>
              {form.save}
            </Button>
          ) : null}
          <Button kind="secondary" size="md" onClick={onClose} disabled={busy}>
            {signing || profile ? 'Close' : 'Cancel'}
          </Button>
        </div>
        <Outcome busy={busy} busyText={source === 'signin' ? 'Starting…' : 'Adding…'} error={error} />
      </Form>
    </Tile>
  );
}

/// A sign-in under way: its stage, and its link whole, wrapped, with a button that opens it in a
/// new tab with no opener and no referrer, and one that copies it. Nothing opens by itself
/// (ST-C44). What the router's terminal printed is never shown (ST-C47).
function SigninPanel({ status, url, misses }: { status: Signin; url: string | null; misses: { readonly count: number; readonly error: unknown } }) {
  const words = signinWords(status);
  return (
    <div className={styles.flow} aria-live="polite">
      {running(status) ? (
        <InlineLoading description={words} status="active" />
      ) : status.stage === 'failed' ? (
        <ReadFailed what="The sign-in did not complete" error={status.why} />
      ) : status.stage === 'done' ? (
        <Explain>{words}</Explain>
      ) : null}
      {url && running(status) ? (
        <>
          <CodeSnippet type="multi" wrapText hideCopyButton className={styles.snippet} aria-label="The sign-in link">
            {url}
          </CodeSnippet>
          <div className={styles.actions}>
            <Button kind="primary" size="md" href={url} target="_blank" rel="noopener noreferrer">
              Open sign-in page
            </Button>
            <CopyButton text={url} label="Copy link" what="the link" />
          </div>
          <Explain>Nothing opens on its own — use the button, or copy the link into whichever browser is signed into that account.</Explain>
        </>
      ) : null}
      {misses.count > 1 ? <ReadFailed what="Could not read the sign-in’s progress; still trying" error={misses.error} /> : null}
    </div>
  );
}
