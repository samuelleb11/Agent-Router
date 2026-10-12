import { useState, type FormEvent } from 'react';
import { Button, ComposedModal, ModalBody, ModalFooter, ModalHeader, Select, SelectItem, TextInput } from '@carbon/react';
import { keys } from '~/app/query';
import { CopyButton } from '~/components/CopyButton';
import { FormModal } from '~/components/FormModal';
import { Page } from '~/components/Page';
import { createAccessToken, revokeAccessToken, useAccessTokens, type CreatedAccessToken } from '~/data/access-tokens';
import { useReread } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { TOKEN_LIFETIMES, TOKEN_SCOPES, tokenExpiry, tokenScopeLabel, tokenStatus, type AccessToken, type TokenLifetime, type TokenScope } from '~/model/access-tokens';
import { Confirm, Explain, Fact, Facts, Outcome, ReadFailed, Reading } from './parts';
import styles from './Settings.module.scss';

export default function AccessTokensPage() {
  const tokens = useAccessTokens();
  const reread = useReread();
  const [creating, setCreating] = useState(false);
  // The one-time secret stays in this page's component state, never in a query or browser store.
  const [created, setCreated] = useState<CreatedAccessToken | null>(null);
  const [revoking, setRevoking] = useState<AccessToken | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState<string | null>(null);
  const refresh = () => reread(keys.auth);
  const revoke = async () => {
    if (!revoking || busy) return;
    setBusy(true);
    setError(null);
    try {
      await revokeAccessToken(revoking.id);
      setDone(`Revoked ${revoking.name}`);
      setRevoking(null);
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
      await refresh();
    }
  };
  return (
    <>
      <Page.Section
        title="Access tokens"
        action={
          <Button kind="tertiary" size="sm" disabled={tokens.value === null} onClick={() => setCreating(true)}>
            Create a token
          </Button>
        }
      >
        <Explain>
          Give each client or machine its own named credential for this router. Choose its permissions and expiry; revoke it without changing other
          clients. Provider API keys stay on the Accounts page.
        </Explain>
        <Explain>
          Clients send the token as <code>Authorization: Bearer &lt;token&gt;</code> or <code>x-api-key</code>. Existing router passwords continue to work.
        </Explain>
        {tokens.stale ? <p className={styles.subtle}>Showing what was read {since(tokens.at)}; the last read failed: {refusalText(tokens.error)}</p> : null}
        <Outcome done={done} />
        {tokens.value === null ? (
          tokens.unreachable ? <ReadFailed what="Could not read access tokens" error={tokens.error} /> : <Reading what="Reading access tokens…" />
        ) : tokens.value.length === 0 ? (
          <p className={styles.lead}>No access tokens yet. Create one for each client or machine you connect.</p>
        ) : (
          <ul className={styles.cards}>
            {tokens.value.map((token) => <TokenCard key={token.id} token={token} onRevoke={() => { setError(null); setDone(null); setRevoking(token); }} />)}
          </ul>
        )}
      </Page.Section>
      {creating ? (
        <CreateToken
          onClose={() => setCreating(false)}
          onCreated={(token) => { setCreating(false); setCreated(token); }}
          refresh={refresh}
        />
      ) : null}
      {created ? <TokenSecret created={created} onClose={() => setCreated(null)} /> : null}
      <Confirm
        open={revoking !== null}
        title={`Revoke ${revoking?.name ?? ''}?`}
        confirmText="Revoke token"
        busy={busy}
        error={error}
        onConfirm={() => void revoke()}
        onClose={() => { if (!busy) setRevoking(null); }}
      >
        <p>This token will stop authenticating new requests immediately. Existing machine connections close within five seconds. Give any client or machine using it a replacement token to reconnect.</p>
      </Confirm>
    </>
  );
}

function TokenCard({ token, onRevoke }: { token: AccessToken; onRevoke: () => void }) {
  const status = tokenStatus(token);
  return (
    <li className={styles.card} data-token={token.id}>
      <h3 className={styles.cardTitle}>{token.name}</h3>
      <Facts label={`Access token ${token.name}`}>
        <Fact term="Token" mono>{token.prefix}…</Fact>
        <Fact term="Permissions">{tokenScopeLabel(token.scope)}</Fact>
        <Fact term="Status"><span className={status === 'Active' ? styles.good : styles.subtle}>{status}</span></Fact>
        <Fact term="Created"><TokenTime at={token.created_at} /></Fact>
        <Fact term="Last used">{token.last_used_at === null ? 'Never used' : <TokenTime at={token.last_used_at} />}</Fact>
        <Fact term="Expires">{token.expires_at === null ? 'Never' : <TokenTime at={token.expires_at} />}</Fact>
        {token.revoked_at !== null ? <Fact term="Revoked at"><TokenTime at={token.revoked_at} /></Fact> : null}
      </Facts>
      <div className={styles.actions}>
        <Button kind="danger--ghost" size="sm" disabled={token.revoked_at !== null} onClick={onRevoke}>Revoke</Button>
      </div>
    </li>
  );
}

function TokenTime({ at }: { at: number }) {
  const date = new Date(at);
  if (!Number.isFinite(date.getTime())) return <span>Unknown date</span>;
  return <time dateTime={date.toISOString()}>{date.toLocaleString()}</time>;
}

function CreateToken({ onClose, onCreated, refresh }: { onClose: () => void; onCreated: (token: CreatedAccessToken) => void; refresh: () => Promise<void> }) {
  const [name, setName] = useState('');
  const [scope, setScope] = useState<TokenScope>('client');
  const [lifetime, setLifetime] = useState<TokenLifetime>('90');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || !name.trim()) return;
    setBusy(true);
    setError(null);
    try {
      onCreated(await createAccessToken(name.trim(), scope, tokenExpiry(lifetime)));
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
      await refresh();
    }
  };
  return (
    <FormModal open title="Create an access token" submitText={busy ? 'Creating…' : 'Create token'} submitDisabled={!name.trim()} busy={busy} focusId="access-token-name" onSubmit={(e) => void submit(e)} onClose={() => { if (!busy) onClose(); }}>
      <TextInput id="access-token-name" labelText="Name" helperText="The client or machine using it, such as Codex on my laptop; up to 64 characters" value={name} maxLength={64} onChange={(e) => setName(e.target.value)} autoComplete="off" />
      <Select id="access-token-scope" labelText="Permissions" value={scope} helperText={TOKEN_SCOPES.find((s) => s.value === scope)?.description} onChange={(e) => setScope(e.target.value as TokenScope)}>
        {TOKEN_SCOPES.map((s) => <SelectItem key={s.value} value={s.value} text={s.label} />)}
      </Select>
      <Select id="access-token-lifetime" labelText="Expires after" value={lifetime} onChange={(e) => setLifetime(e.target.value as TokenLifetime)}>
        {TOKEN_LIFETIMES.map((l) => <SelectItem key={l.value} value={l.value} text={l.label} />)}
      </Select>
      <Explain>The token is shown once after creation. Save it in your client’s configuration before closing the next dialog.</Explain>
      <Outcome busy={busy} busyText="Creating token…" error={error} refusedTitle="Could not complete token creation" />
    </FormModal>
  );
}

function TokenSecret({ created, onClose }: { created: CreatedAccessToken; onClose: () => void }) {
  return (
    <ComposedModal open size="sm" aria-label="Save your access token" preventCloseOnClickOutside onClose={onClose}>
      <ModalHeader title="Save your access token" label={created.name} />
      <ModalBody>
        <div className={styles.block}>
          <Explain>This is the only time the full token is shown. Copy it to your client or machine configuration before closing.</Explain>
          <code className={styles.path}>{created.token}</code>
          <CopyButton text={created.token} label="Copy token" what="the token" />
          <Explain>Permissions: {tokenScopeLabel(created.scope)}. {created.expires_at === null ? 'It does not expire.' : `Expires ${new Date(created.expires_at).toLocaleString()}.`}</Explain>
        </div>
      </ModalBody>
      <ModalFooter><Button onClick={onClose}>I have saved the token</Button></ModalFooter>
    </ComposedModal>
  );
}
