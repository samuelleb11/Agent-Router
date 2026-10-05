// New for agent-router: one account, as a card that carries every fact a desk row had and its
// actions at rest — Use, Edit, Remove — with the editor opening inside it (assets/index.html,
// `renderAccounts`, `openEdit`, `wireEditForm`; ST-C19 to ST-C38).
import { useState, type FormEvent, type KeyboardEvent } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Button, Form, PasswordInput, Tag, TextInput, Tile } from '@carbon/react';
import { keys } from '~/app/query';
import { activateAccount, clearSessionKey, editAccount, removeAccount, setSessionKey, useReread, type AccountEdit, type AccountRow } from '~/data/settings';
import { identityNote, notes, removable, type Auth } from '~/model/settings';
import {
  accountName,
  BUILT_IN,
  credential,
  expiry,
  flag,
  HELD,
  identityProblem,
  NOT_ON_A_SERVER,
  removeWords,
  sessionKeyProblem,
  sourceWord,
  tokenProblem,
  usable,
  USAGE_KEY,
} from './rules';
import { Confirm, Explain, Fact, Facts, Outcome, Words } from './parts';
import styles from './Settings.module.scss';

export function AccountCard({ account, accounts, auth, hosted }: { account: AccountRow; accounts: readonly AccountRow[]; auth: Auth | null; hosted: boolean }) {
  const reread = useReread();
  const [editing, setEditing] = useState(false);
  const [asking, setAsking] = useState(false);
  const active = flag(account.active);
  const name = accountName(account);
  const cred = credential(account);
  const builtIn = account.source === 'keychain' && !hosted;

  // Use and Remove each re-read the list whatever the router said, and a refusal stays on this
  // card in the router's words (ST-C24, ST-C25).
  const use = useMutation({
    mutationFn: async () => {
      try {
        await activateAccount(account.id);
      } finally {
        await reread(keys.accounts);
      }
    },
  });
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
        <h3 className={styles.cardTitle}>{name}</h3>
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
        <Fact term="Org">{account.org_name || '—'}</Fact>
        <Fact term="Plan">{account.subscription_type || '—'}</Fact>
        <Fact term="Source">
          <span className={styles.inline}>
            <Tag type="gray" size="sm">
              {sourceWord(account.source)}
            </Tag>
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
        <Fact term="Expires">{expiry(account)}</Fact>
      </Facts>

      {active ? <Explain>Signs every request now.</Explain> : null}
      {!usable(account, hosted) ? (
        <Explain className={styles.bad}>
          It cannot sign on this router: {cred.kind === 'failed' ? cred.text : NOT_ON_A_SERVER}
        </Explain>
      ) : null}

      <div className={styles.actions}>
        {!active && usable(account, hosted) ? (
          <Button kind="primary" size="md" onClick={() => use.mutate()} disabled={use.isPending}>
            Use
          </Button>
        ) : null}
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
          <span className={styles.subtle}>{BUILT_IN}</span>
        </div>
      ) : null}
      <Outcome busy={use.isPending} busyText="Switching…" error={use.error} />
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

  const builtIn = account.source === 'keychain';
  const ownsToken = account.source === 'token';
  const hasKey = flag(account.usage_key);
  const id = `edit-${account.id}`;

  const edit: { -readonly [K in keyof AccountEdit]: AccountEdit[K] } = {};
  if (label !== undefined && label.trim() !== '' && label.trim() !== (account.label ?? '')) edit.label = label.trim();
  if (!builtIn && email !== undefined && email.trim() !== '' && email.trim() !== (account.email ?? '')) edit.email = email.trim();
  if (ownsToken && token.trim() !== '') edit.token = token.trim();
  const newKey = key.trim();
  const problems = {
    email: builtIn || email === undefined ? null : identityProblem(email),
    token: ownsToken ? tokenProblem(token) : null,
    key: sessionKeyProblem(key),
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
      {ownsToken ? (
        <PasswordInput
          id={`${id}-token`}
          labelText="Credential"
          autoComplete="new-password"
          spellCheck={false}
          placeholder={flag(account.has_token) ? HELD : 'sk-ant-oat01-…'}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          invalid={tried && !!problems.token}
          invalidText={problems.token ?? undefined}
          helperText={
            <>
              {tokenState} A fresh <code>claude setup-token</code> value, if the current one was revoked or you want to rotate it. The account keeps its
              id, so everything already recorded against it stays intact.
            </>
          }
        />
      ) : null}
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
