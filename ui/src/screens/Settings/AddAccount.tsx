// New for agent-router: adding an account, in place under the Accounts heading (assets/index.html,
// `wireAddForm`; ST-C39 to ST-C52, ST-C117, ST-C118, ST-C124). The provider is asked first, each a
// choice with its mark, its name and its explanation; then, for Anthropic, each way to add one is a
// choice that carries its explanation, or, when the router offers one way only, that way's field
// with its explanation; Kimi Code goes straight to its key. The sign-in link and the profile command
// sit in code snippets with a copy button that says when copying failed; nothing opens a window by
// itself.
import { useEffect, useId, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { Button, CodeSnippet, Form, InlineLoading, PasswordInput, RadioTile, TextInput, Tile, TileGroup } from '@carbon/react';
import { keys } from '~/app/query';
import { CopyButton } from '~/components/CopyButton';
import { ProviderMark } from '~/components/ProviderMark';
import { addAccount, probeProfile, readLogin, startSignin, useReread, type AccountRow } from '~/data/settings';
import { accountSources, ADD_PROVIDERS, addForm, KIMI_BASE_URL, OPENAI_BASE_URL, notes, providerName, type AddProvider, type Auth, type Source } from '~/model/settings';
import {
  accountId,
  baseUrlProblem,
  keyProblem,
  looksLikeEmail,
  nameProblem,
  PROFILE_POLL_MS,
  running,
  signin,
  SIGNIN_POLL_MS,
  signinWords,
  TOKEN_NEEDED,
  tokenProblem,
  type Signin,
} from './rules';
import { Explain, Outcome, ReadFailed, Words } from './parts';
import styles from './Settings.module.scss';

/// An explanation's words: text, and the code and emphasis inside it, never markup.
type Parts = readonly (string | { readonly code: string } | { readonly b: string })[];

/// Each provider, as its choice explains it; its mark and name are the shared ones (ST-C124, SH-C56).
const PROVIDER: Readonly<Record<AddProvider, Parts>> = {
  anthropic: ['Claude’s models, signed by an Anthropic API key or a Claude subscription.'],
  kimi: ['Kimi’s models (', { code: 'k3' }, ', ', { code: 'kimi-for-coding' }, '), signed by a key from the Kimi Code console.'],
  openai: ['OpenAI’s models, signed by an OpenAI API key. On a local router, the Codex login is discovered automatically.'],
};

/// Each of Anthropic's ways in, as its choice says it (ST-C40, ST-C43).
const CHOICE: Readonly<Record<Source, { readonly title: string; readonly body: Parts }>> = {
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
    body: ['An Anthropic API key, or the subscription value ', { code: 'claude setup-token' }, ' prints, ', { code: 'sk-ant-oat01-…' }, ', signed in as the account to add.'],
  },
  profile: {
    title: 'Separate Claude Code profile',
    body: [addForm('profile').help ?? ''],
  },
};

function Body({ parts }: { parts: Parts }) {
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
  // Anthropic first, chosen as the form opens, on every router (ST-C124).
  const [provider, setProvider] = useState<AddProvider>(ADD_PROVIDERS[0]);
  const kimi = provider === 'kimi';
  const keyed = provider !== 'anthropic';
  const defaultBase = kimi ? KIMI_BASE_URL : OPENAI_BASE_URL;
  const sources = accountSources(auth);
  const offered = sources.join(',');
  const [picked, setPicked] = useState<Source>(sources[0] ?? 'token');
  // A choice the router no longer offers (it said it is hosted) gives way to the first one left.
  const source: Source = sources.includes(picked) ? picked : (sources[0] ?? 'token');
  // Anthropic's way, or none for Kimi Code, which has its key only.
  const way: Source | null = keyed ? null : source;
  const [label, setLabel] = useState('');
  const [token, setToken] = useState('');
  // A Kimi Code key has fields of its own: a token typed for Anthropic is not one.
  const [providerKeys, setProviderKeys] = useState<Partial<Record<AddProvider, string>>>({});
  const [providerUrls, setProviderUrls] = useState<Partial<Record<AddProvider, string>>>({});
  const key = providerKeys[provider] ?? '';
  const baseUrl = providerUrls[provider] ?? '';
  const setKey = (value: string) => setProviderKeys((old) => ({ ...old, [provider]: value }));
  const setBaseUrl = (value: string) => setProviderUrls((old) => ({ ...old, [provider]: value }));
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [profile, setProfile] = useState<{ readonly id: string; readonly command: string } | null>(null);
  const flow = useSignin(way === 'signin');
  const groupName = useId();
  const form = addForm(source);

  const nameIssue = nameProblem(label, accounts);
  // The name is checked as it is typed once it says something; empty, only once Add was pressed.
  const showName = (label.trim() !== '' || tried) && !!nameIssue;
  // Each provider's key by its own rule: `sk-ant-` is Anthropic's alone (ST-C118).
  const tokenIssue = way === 'token' ? (token.trim() === '' ? TOKEN_NEEDED : tokenProblem(token)) : null;
  const keyIssue = keyed ? (key.trim() === '' ? TOKEN_NEEDED : keyProblem(provider, key)) : null;
  const urlIssue = keyed ? baseUrlProblem(baseUrl) : null;
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
    if (busy || nameIssue || tokenIssue || keyIssue || urlIssue) return;
    setBusy(true);
    setError(null);
    try {
      if (way === null) {
        // Sent as it always was (ST-C117).
        await addAccount(label, { source: kimi ? 'kimi' : 'openai', token: key, base_url: baseUrl });
        setKey('');
        finish(`Added ${label.trim()}.`);
      } else if (way === 'signin') {
        flow.begin(signin(await startSignin(label)));
      } else if (way === 'token') {
        await addAccount(label, { source: 'token', token });
        // Stored: the secret leaves the screen. On a refusal it stays where it was pasted (ST-C50).
        setToken('');
        finish(`Added ${label.trim()}.`);
      } else {
        const added = await addAccount(label, { source: 'profile' });
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

  const signing = way === 'signin' && flow.seen && running(flow.status);
  const url = flow.status.stage === 'waiting' || flow.status.stage === 'starting' ? flow.status.url : null;

  return (
    <Tile className={styles.addForm}>
      <Form aria-label="Add an account" onSubmit={(e) => void submit(e)} onKeyDown={escape} className={styles.form}>
        <h3 className={styles.cardTitle}>Add an account</h3>
        <Explain>Add a provider credential here. Client and Machine access tokens authenticate to Agent Router and are created on the Access tokens page.</Explain>
        {/* The provider first, each by its mark and name, then its explanation (ST-C124). */}
        <TileGroup legend="Provider" name={`${groupName}-provider`} valueSelected={provider} onChange={(v) => setProvider(v as AddProvider)} className={styles.choices}>
          {ADD_PROVIDERS.map((p) => (
            <RadioTile key={p} id={`${groupName}-provider-${p}`} value={p} className={styles.choice}>
              <span className={styles.choiceTitle}>
                <ProviderMark provider={p} />
              </span>
              <span className={styles.choiceBody}>
                <Body parts={PROVIDER[p]} />
              </span>
            </RadioTile>
          ))}
        </TileGroup>

        {/* Anthropic's ways, when the router offers more than one: a hosted router's one way, the
            setup token, is no choice, and its field below explains it (ST-C124, ST-C48). */}
        {!keyed && sources.length > 1 ? (
          <TileGroup key={offered} legend="How to add it" name={`${groupName}-way`} valueSelected={source} onChange={(v) => setPicked(v as Source)} className={styles.choices}>
            {sources.map((s) => (
              <RadioTile key={s} id={`${groupName}-way-${s}`} value={s} className={styles.choice}>
                <span className={styles.choiceTitle}>{CHOICE[s].title}</span>
                <span className={styles.choiceBody}>
                  <Body parts={CHOICE[s].body} />
                </span>
              </RadioTile>
            ))}
          </TileGroup>
        ) : null}

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
            (keyed || way === 'token') && looksLikeEmail(label)
              ? 'This name is an email address, so it becomes the account’s identity: a long-lived token cannot report which account it belongs to.'
              : id
                ? `Its id will be “${id}”.`
                : 'What the account is called here.'
          }
          disabled={signing}
        />

        {way === 'token' ? (
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
            helperText={<><span>An Anthropic API key also works. </span><Words segments={notes(auth).token} /></>}
          />
        ) : null}

        {keyed ? (
          <>
            <PasswordInput
              id={`${groupName}-key-field`}
              labelText={kimi ? 'Key' : 'OpenAI API key'}
              placeholder={kimi ? 'the key from the Kimi Code console' : 'sk-proj-…'}
              autoComplete="new-password"
              spellCheck={false}
              value={key}
              onChange={(e) => setKey(e.target.value)}
              invalid={tried && !!keyIssue}
              invalidText={keyIssue ?? undefined}
              helperText={kimi ? 'Made in the Kimi Code console, signed in to your plan. An Anthropic token is not one: it cannot sign Kimi Code’s requests.' : 'Made in your OpenAI API project. The router uses this key to sign requests to OpenAI.'}
            />
            <TextInput
              id={`${groupName}-base-url-field`}
              labelText="Base URL (optional)"
              placeholder={defaultBase}
              autoComplete="off"
              spellCheck={false}
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              invalid={tried && !!urlIssue}
              invalidText={urlIssue ?? undefined}
              helperText={`Where its requests go. Left blank, ${providerName(provider)}’s own: ${defaultBase}.`}
            />
          </>
        ) : null}

        {way === 'signin' && flow.seen ? <SigninPanel status={flow.status} url={url} misses={flow.misses} /> : null}

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
              {kimi ? 'Add account' : form.save}
            </Button>
          ) : null}
          <Button kind="secondary" size="md" onClick={onClose} disabled={busy}>
            {signing || profile ? 'Close' : 'Cancel'}
          </Button>
        </div>
        <Outcome busy={busy} busyText={way === 'signin' ? 'Starting…' : 'Adding…'} error={error} />
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
