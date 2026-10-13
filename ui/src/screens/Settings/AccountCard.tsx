// New for agent-router: one account, as a card that carries every fact a desk row had and its
// actions at rest — Edit, Remove — with the editor opening inside it (assets/index.html,
// `renderAccounts`, `openEdit`, `wireEditForm`; ST-C19 to ST-C38). It names its provider, and Org,
// Plan and the claude.ai session key are Anthropic's alone (ST-C119, ST-C120). Which account signs
// is chosen in its provider's section, above the cards (ST-C125). A ChatGPT login says its plan, that it
// serves Codex only, and takes a fresh paste from Edit (AGEN-0193).
import { useState, type FormEvent, type KeyboardEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Button, Form, PasswordInput, Tag, TextArea, TextInput, Tile } from '@carbon/react';
import { keys } from '~/app/query';
import { clearSessionKey, editAccount, removeAccount, setSessionKey, useReread, type AccountEdit, type AccountRow } from '~/data/settings';
import { identityNote, KIMI_BASE_URL, OPENAI_BASE_URL, notes, providerName, providerOf, removable, type Auth } from '~/model/settings';
import {
  accountName,
  activeWords,
  BUILT_IN,
  CHATGPT_REFRESHED,
  CHATGPT_SERVES,
  chatgptLoginProblem,
  CODEX_BUILT_IN,
  CODEX_NOT_ON_A_SERVER,
  credential,
  expiry,
  flag,
  HELD,
  identityProblem,
  keyProblem,
  NOT_ON_A_SERVER,
  removeWords,
  sessionKeyProblem,
  sourceWord,
  usable,
  USAGE_KEY,
} from './rules';
import { ChatGPTSteps } from './AddAccount';
import { Confirm, Explain, Fact, Facts, Outcome, Words } from './parts';
import styles from './Settings.module.scss';

export function AccountCard({ account, accounts, auth, hosted }: { account: AccountRow; accounts: readonly AccountRow[]; auth: Auth | null; hosted: boolean }) {
  const reread = useReread();
  const [editing, setEditing] = useState(false);
  const [asking, setAsking] = useState(false);
  const active = flag(account.active);
  const name = accountName(account);
  const cred = credential(account);
  const builtIn = (account.source === 'keychain' || account.source === 'codex') && !hosted;
  const anthropic = providerOf(account) === 'anthropic';
  const chatgpt = account.source === 'chatgpt';

  // Remove re-reads the list whatever the router said, and a refusal stays on this card in the
  // router's words (ST-C25).
  const remove = useMutation({
    mutationFn: async () => {
      try {
        await removeAccount(account.id);
      } finally {
        await reread(keys.accounts);
      }
    },
  });

  return (
    <Tile className={styles.card} aria-label={`Account ${name}`}>
      <div className={styles.cardHead}>
        <h4 className={styles.cardTitle}>{name}</h4>
        {active ? (
          <Tag type="green" size="md">
            active
          </Tag>
        ) : null}
      </div>
      <Facts label={`What the router knows about ${name}`}>
        {account.email && account.label && account.label !== account.email ? <Fact term="Name">{account.label}</Fact> : null}
        <Fact term="Id" mono>
          {account.id}
        </Fact>
        <Fact term="Provider">{providerName(providerOf(account))}</Fact>
        {/* Org and plan are what Anthropic reports of a login; a Kimi Code key reports neither,
            and says where its requests go instead (ST-C119). A ChatGPT login says its plan, and
            goes to ChatGPT's own backend. */}
        {anthropic ? (
          <>
            <Fact term="Org">{account.org_name || '—'}</Fact>
            <Fact term="Plan">{account.subscription_type || '—'}</Fact>
          </>
        ) : chatgpt ? (
          <Fact term="Plan">{account.subscription_type || '—'}</Fact>
        ) : account.source !== 'codex' ? (
          <Fact term="Base URL" mono>
            {account.base_url || (providerOf(account) === 'kimi' ? KIMI_BASE_URL : providerOf(account) === 'openai' ? OPENAI_BASE_URL : '—')}
          </Fact>
        ) : null}
        <Fact term="Source">
          <span className={styles.inline}>
            <Tag type="gray" size="sm">
              {sourceWord(account.source)}
            </Tag>
            {chatgpt ? (
              <Tag type="purple" size="sm">
                {CHATGPT_SERVES}
              </Tag>
            ) : null}
            {flag(account.usage_key) ? (
              <Tag type="blue" size="sm">
                usage key
              </Tag>
            ) : null}
          </span>
          {flag(account.usage_key) ? <span className={styles.subtle}>{USAGE_KEY}</span> : null}
        </Fact>
        <Fact term="Credential">
          <span className={cred.kind === 'failed' ? styles.bad : cred.kind === 'unchecked' ? styles.subtle : undefined} data-credential={cred.kind}>
            {cred.text}
          </span>
        </Fact>
        {/* A ChatGPT login lasts past its access token, which the router refreshes. */}
        <Fact term={chatgpt ? 'Access token expires' : 'Expires'}>{expiry(account)}</Fact>
      </Facts>

      {active ? <Explain>{activeWords(account)}</Explain> : null}
      {chatgpt ? <Explain>{CHATGPT_REFRESHED}</Explain> : null}
      {!usable(account, hosted) ? (
        <Explain className={styles.bad}>
          It cannot sign on this router: {cred.kind === 'failed' ? cred.text : account.source === 'codex' ? CODEX_NOT_ON_A_SERVER : NOT_ON_A_SERVER}
        </Explain>
      ) : null}

      <div className={styles.actions}>
        <Button kind="secondary" size="md" onClick={() => setEditing(true)} disabled={editing}>
          Edit
        </Button>
        {removable(account, auth) ? (
          <Button
            kind="danger--tertiary"
            size="md"
            onClick={() => {
              remove.reset();
              setAsking(true);
            }}
          >
            Remove
          </Button>
        ) : null}
      </div>
      {builtIn ? (
        <div className={styles.builtIn}>
          <Tag type="cool-gray" size="sm">
            built in
          </Tag>
          <span className={styles.subtle}>{account.source === 'codex' ? CODEX_BUILT_IN : BUILT_IN}</span>
        </div>
      ) : null}
      <Outcome error={remove.error} />

      {editing ? <AccountEditor account={account} auth={auth} onClose={() => setEditing(false)} /> : null}

      <Confirm
        open={asking}
        title={`Remove ${name}?`}
        label="Accounts"
        confirmText="Remove account"
        busy={remove.isPending}
        onClose={() => setAsking(false)}
        onConfirm={() => {
          // The answer, either way, is shown on the card, which is where the reader returns to.
          remove.mutate(undefined, { onSettled: () => setAsking(false) });
        }}
      >
        {removeWords(accounts, account.id).map((s) => (
          <p key={s}>{s}</p>
        ))}
      </Confirm>
    </Tile>
  );
}

/// The editor, inside the account's card (ST-C29). Every field starts as the account is now and
/// follows it until the reader types; only what the reader changed is sent, by the account's id,
/// never a copy of the row taken when it opened (ST-C14, ST-C34, ST-C35). Enter saves and Escape
/// cancels. A refused save keeps everything typed, and it closes once what was saved reads back.
function AccountEditor({ account, auth, onClose }: { account: AccountRow; auth: Auth | null; onClose: () => void }) {
  const reread = useReread();
  // Undefined until the reader types, so an untouched field keeps following the row.
  const [label, setLabel] = useState<string | undefined>(undefined);
  const [email, setEmail] = useState<string | undefined>(undefined);
  const [token, setToken] = useState('');
  const [key, setKey] = useState('');
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [fieldsError, setFieldsError] = useState<unknown>(null);
  const [keyError, setKeyError] = useState<unknown>(null);
  const [saved, setSaved] = useState<string[]>([]);
  const [clearing, setClearing] = useState(false);
  const clear = useMutation({
    mutationFn: async () => {
      await clearSessionKey(account.id);
      await reread(keys.accounts);
    },
    onSuccess: () => setClearing(false),
  });

  const builtIn = account.source === 'keychain' || account.source === 'codex';
  const chatgptLogin = account.source === 'chatgpt';
  const ownsToken = account.source === 'token' || chatgptLogin;
  const provider = providerOf(account);
  const kimi = provider === 'kimi';
  // A claude.ai session key reads Anthropic's usage; a Kimi Code account has none to read (ST-C120).
  const takesSessionKey = provider === 'anthropic';
  const hasKey = flag(account.usage_key);
  const id = `edit-${account.id}`;

  const edit: { -readonly [K in keyof AccountEdit]: AccountEdit[K] } = {};
  if (label !== undefined && label.trim() !== '' && label.trim() !== (account.label ?? '')) edit.label = label.trim();
  if (!builtIn && email !== undefined && email.trim() !== '' && email.trim() !== (account.email ?? '')) edit.email = email.trim();
  if (ownsToken && token.trim() !== '') edit.token = token.trim();
  const newKey = takesSessionKey ? key.trim() : '';
  const problems = {
    email: builtIn || email === undefined ? null : identityProblem(email),
    // By the account's own provider's rule: a Kimi Code key is never `sk-ant-` (ST-C118, ST-C120).
    token: chatgptLogin ? chatgptLoginProblem(token) : ownsToken ? keyProblem(provider, token) : null,
    key: takesSessionKey ? sessionKeyProblem(key) : null,
  };
  const nothing = Object.keys(edit).length === 0 && newKey === '';

  const save = async (e?: FormEvent) => {
    e?.preventDefault();
    setTried(true);
    if (busy || nothing || problems.email || problems.token || problems.key) return;
    setBusy(true);
    setFieldsError(null);
    setKeyError(null);
    setSaved([]);
    const done: string[] = [];
    let fieldsOk = true;
    let keyOk = true;
    // Two requests, each with its own outcome: a refused key leaves the saved fields saved (ST-C36).
    if (Object.keys(edit).length > 0) {
      try {
        await editAccount(account.id, edit);
        if (edit.label) done.push('name');
        if (edit.email) done.push('identity');
        if (edit.token) done.push('credential');
        // Saved: they follow the row again, and a stored token leaves the screen.
        if (edit.label) setLabel(undefined);
        if (edit.email) setEmail(undefined);
        if (edit.token) setToken('');
      } catch (err) {
        fieldsOk = false;
        setFieldsError(err);
      }
    }
    if (newKey !== '') {
      try {
        await setSessionKey(account.id, newKey);
        done.push('session key');
        setKey('');
      } catch (err) {
        keyOk = false;
        setKeyError(err);
      }
    }
    // The list re-reads either way, and the editor closes once what was saved reads back.
    await reread(keys.accounts);
    setBusy(false);
    setSaved(done);
    if (fieldsOk && keyOk) onClose();
  };

  const escape = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && !busy && !clearing) {
      e.stopPropagation();
      onClose();
    }
  };

  const tokenState = account.has_token == null ? 'Not checked; leave it blank to keep whatever is stored.' : flag(account.has_token) ? 'Stored; leave it blank to keep it.' : 'None stored.';

  return (
    <Form className={styles.editor} aria-label={`Edit ${accountName(account)}`} onSubmit={(e) => void save(e)} onKeyDown={escape}>
      <div className={styles.fields}>
        <TextInput
          id={`${id}-label`}
          labelText="Name"
          autoComplete="off"
          value={label ?? account.label ?? ''}
          onChange={(e) => setLabel(e.target.value)}
          helperText="Blank keeps the name it has."
        />
        <TextInput
          id={`${id}-email`}
          labelText="Identity"
          autoComplete="off"
          spellCheck={false}
          placeholder="you@example.com"
          readOnly={builtIn}
          value={builtIn ? (account.email ?? '') : (email ?? account.email ?? '')}
          onChange={(e) => setEmail(e.target.value)}
          invalid={tried && !!problems.email}
          invalidText={problems.email ?? undefined}
          helperText={identityNote(account, auth)}
        />
      </div>
      {chatgptLogin ? (
        <div className={styles.field}>
          <ChatGPTSteps />
          <TextArea
            id={`${id}-login`}
            labelText="New ChatGPT login"
            rows={4}
            spellCheck={false}
            autoComplete="off"
            className={styles.secretBox}
            placeholder={flag(account.has_token) ? HELD : '{"auth_mode": "chatgpt", "tokens": { … } }'}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            invalid={(tried || token.trim() !== '') && !!problems.token}
            invalidText={problems.token ?? undefined}
            helperText={`${tokenState} A login from a new Codex sign-in replaces the stored one, which is what mends a login OpenAI refused to refresh. The account keeps its id, so everything already recorded against it stays intact.`}
          />
        </div>
      ) : ownsToken ? (
        <PasswordInput
          id={`${id}-token`}
          labelText="Credential"
          autoComplete="new-password"
          spellCheck={false}
          placeholder={flag(account.has_token) ? HELD : kimi ? 'the key from the Kimi Code console' : provider === 'openai' ? 'sk-proj-…' : 'sk-ant-oat01-…'}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          invalid={tried && !!problems.token}
          invalidText={problems.token ?? undefined}
          helperText={
            kimi ? (
              <>
                {tokenState} A fresh key from the Kimi Code console, if the current one was revoked or you want to rotate it. The account keeps its id, so
                everything already recorded against it stays intact.
              </>
            ) : provider === 'openai' ? (
              <>{tokenState} A fresh OpenAI API key replaces the stored key and keeps this account’s recorded traffic.</>
            ) : (
              <>
                {tokenState} A fresh <code>claude setup-token</code> value, if the current one was revoked or you want to rotate it. The account keeps its
                id, so everything already recorded against it stays intact.
              </>
            )
          }
        />
      ) : null}
      {takesSessionKey ? (
        <div className={styles.field}>
          <PasswordInput
            id={`${id}-key`}
            labelText="claude.ai session key"
            autoComplete="new-password"
            spellCheck={false}
            placeholder={hasKey ? HELD : 'sk-ant-sid…'}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            invalid={tried && !!problems.key}
            invalidText={problems.key ?? undefined}
            helperText={
              <>
                {hasKey ? 'Stored; leave it blank to keep it, or type a new one to replace it. ' : 'None stored; optional, for usage figures. '}
                <Words segments={notes(auth).usageKey} />
              </>
            }
          />
          {keyError ? <Outcome error={keyError} refusedTitle="The session key was refused" /> : null}
          {hasKey ? (
            <div className={styles.actions}>
              <Button
                kind="danger--tertiary"
                size="md"
                onClick={() => {
                  clear.reset();
                  setClearing(true);
                }}
              >
                Remove stored key
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
      <div className={styles.actions}>
        <Button type="submit" kind="primary" size="md" disabled={busy || nothing}>
          Save
        </Button>
        <Button kind="secondary" size="md" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
      </div>
      <Outcome busy={busy} busyText="Saving…" error={fieldsError} done={!busy && saved.length > 0 ? `Saved the ${saved.join(' and ')}.` : null} />
      <Confirm
        open={clearing}
        title="Remove the stored session key?"
        label={accountName(account)}
        confirmText="Remove stored key"
        busy={clear.isPending}
        error={clear.error}
        onClose={() => setClearing(false)}
        onConfirm={() => clear.mutate()}
      >
        <p>
          The claude.ai session key stored for {accountName(account)} is deleted. Its usage is no longer read from claude.ai until a new key is
          stored here; the credential that signs its requests is not touched.
        </p>
      </Confirm>
    </Form>
  );
}
