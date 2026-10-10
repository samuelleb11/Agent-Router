// New for agent-router: Settings → Library (docs/library.md, "Forges and sources", "Forge webhooks",
// "Agents publishing"; ST-C109 to ST-C116). The sources the Library imports from, each with its trust,
// its last import and its webhook; the publish tokens a pipeline or a cloud session publishes with,
// each shown once; and whether publishes from connected machines go live.
import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Button, Checkbox, Form, Select, SelectItem, TextInput, Toggle } from '@carbon/react';
import { CopyButton } from '~/components/CopyButton';
import { Page } from '~/components/Page';
import { useRead } from '~/data/held';
import {
  createToken,
  deleteSource,
  importSource,
  newSecret,
  putSource,
  readForges,
  readLibrarySettings,
  readSources,
  readTokens,
  revokeToken,
  setMachinePublishes,
  settingKeys,
} from '~/data/library';
import { since } from '~/lib/freshness';
import { when } from '~/model/format';
import { importWords, isKey, lastWords, scopeWords, type SourceRow, type TokenRow } from '~/model/forges';
import { KINDS } from '~/model/library';
import { Confirm, Explain, Fact, Facts, Outcome, ReadFailed, Reading } from './parts';
import styles from './Settings.module.scss';

export default function LibraryPage() {
  return (
    <>
      <Sources />
      <Tokens />
      <MachinePublishes />
    </>
  );
}

// ---------- sources ----------

function Sources() {
  const client = useQueryClient();
  const sources = useRead({ queryKey: settingKeys.sources, queryFn: ({ signal }) => readSources(signal), retry: false, staleTime: 0 });
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<SourceRow | null>(null);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const reread = () => client.invalidateQueries({ queryKey: ['library'] });
  const remove = async () => {
    if (!removing) return;
    setBusy(true);
    setRefused(null);
    try {
      await deleteSource(removing.key);
      setRemoving(null);
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await reread();
    }
  };
  return (
    <Page.Section
      title="Sources"
      action={
        <Button kind="tertiary" size="sm" onClick={() => setAdding(true)}>
          Add a source
        </Button>
      }
    >
      <Explain>
        A repository, a ref and a path on a forge. Every skill directory, <code>commands/*.md</code> and <code>agents/*.md</code> under the path is an item, imported
        when the ref moves — every five minutes, or at once from the forge’s webhook.
      </Explain>
      {adding && <AddSource onDone={() => setAdding(false)} reread={reread} />}
      {sources.value === null ? (
        sources.unreachable ? <ReadFailed what="Could not read the sources" error={sources.error} /> : <Reading what="Reading the sources…" />
      ) : sources.value.length === 0 ? (
        <p className={styles.lead}>No source yet.</p>
      ) : (
        <ul className={styles.cards}>
          {sources.value.map((s) => (
            <SourceCard key={s.key} source={s} reread={reread} onRemove={() => setRemoving(s)} />
          ))}
        </ul>
      )}
      <Confirm
        open={removing !== null}
        title={`Remove the source ${removing?.key ?? ''}?`}
        confirmText="Remove"
        busy={busy}
        error={refused}
        onConfirm={remove}
        onClose={() => {
          setRemoving(null);
          setRefused(null);
        }}
      >
        <p>Its items stay in the Library, and become anyone’s to change: uploads and publishes may add to them from then on.</p>
      </Confirm>
    </Page.Section>
  );
}

/// One source: where it imports from, its trust, its last import and its webhook (ST-C109), with
/// an import now (ST-C111) and a webhook secret made on request and shown once (ST-C112).
function SourceCard({ source, reread, onRemove }: { source: SourceRow; reread: () => Promise<void>; onRemove: () => void }) {
  const [busy, setBusy] = useState<'import' | 'secret' | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [refused, setRefused] = useState<unknown>(null);
  const [secret, setSecret] = useState<{ secret: string; where: string } | null>(null);
  const run = async (what: 'import' | 'secret') => {
    setBusy(what);
    setRefused(null);
    setDone(null);
    try {
      if (what === 'import') {
        setDone(importWords((await importSource(source.key)).imported));
      } else {
        const made = await newSecret(source.key);
        setSecret({ secret: made.secret, where: made.url ?? `${location.origin}${made.path}` });
      }
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(null);
      await reread();
    }
  };
  const at = source.imported_at ?? source.checked_at;
  return (
    <li className={styles.card} data-source={source.key}>
      <Facts label={`The source ${source.key}`}>
        <Fact term="Source">{source.key}</Fact>
        <Fact term="Forge">{source.forge}</Fact>
        <Fact term="Repository" mono>
          {source.repository}
        </Fact>
        <Fact term="Ref" mono>
          {source.ref}
        </Fact>
        <Fact term="Path" mono>
          {source.path || 'the whole repository'}
        </Fact>
        <Fact term="Its new versions">{source.live ? 'go live' : 'wait for approval'}</Fact>
        <Fact term="Last import">{`${lastWords(source)}${at ? ` (${since(at)})` : ''}`}</Fact>
        <Fact term="Webhook">{source.hook ? 'a secret is set' : 'no secret yet'}</Fact>
      </Facts>
      <div className={styles.actions}>
        <Button kind="tertiary" size="sm" disabled={busy !== null} onClick={() => void run('import')}>
          Import now
        </Button>
        <Button kind="ghost" size="sm" disabled={busy !== null} onClick={() => void run('secret')}>
          {source.hook ? 'Make a new webhook secret' : 'Make a webhook secret'}
        </Button>
        <Button kind="danger--ghost" size="sm" onClick={onRemove}>
          Remove
        </Button>
      </div>
      <Outcome busy={busy !== null} busyText={busy === 'import' ? 'Importing…' : 'Making a secret…'} error={refused} done={done} />
      {secret && (
        <div className={styles.secretBox} data-secret="">
          <p className={styles.lead}>
            Give the forge both — GitLab’s “Secret token”, or GitHub’s “Secret” with content type application/json. The secret is shown now and never again, and any
            secret made before it no longer works.
          </p>
          <Facts label="The webhook">
            <Fact term="URL" mono>
              {secret.where}
            </Fact>
            <Fact term="Secret" mono>
              {secret.secret}
            </Fact>
          </Facts>
          <div className={styles.actions}>
            <CopyButton text={secret.where} what="the URL" />
            <CopyButton text={secret.secret} what="the secret" />
          </div>
        </div>
      )}
    </li>
  );
}

/// Adding a source: its key, its forge (chosen from the forges), repository, ref and path, and
/// whether its new versions go live — off unless the owner says so (ST-C110).
function AddSource({ onDone, reread }: { onDone: () => void; reread: () => Promise<void> }) {
  const forges = useRead({ queryKey: settingKeys.forges, queryFn: ({ signal }) => readForges(signal), retry: false, staleTime: 0 });
  const [key, setKey] = useState('');
  const [forge, setForge] = useState('');
  const [repository, setRepository] = useState('');
  const [reference, setReference] = useState('main');
  const [path, setPath] = useState('');
  const [live, setLive] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const chosen = forge || forges.value?.[0]?.key || '';
  const save = async () => {
    setBusy(true);
    setRefused(null);
    try {
      await putSource(key, { forge: chosen, repository: repository.trim(), ref: reference.trim(), path: path.trim(), live });
      onDone();
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await reread();
    }
  };
  if (forges.value !== null && forges.value.length === 0) {
    return <p className={styles.lead}>Add a forge first, on Settings → Forges: a source imports from one.</p>;
  }
  return (
    <Form
      className={styles.form}
      aria-label="Add a source"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <TextInput id="source-key" labelText="Key" value={key} onChange={(e) => setKey(e.target.value)} invalid={key !== '' && !isKey(key)} invalidText="Lowercase letters and digits, with single hyphens between them" />
      <Select id="source-forge" labelText="Forge" value={chosen} onChange={(e) => setForge(e.target.value)}>
        {(forges.value ?? []).map((f) => (
          <SelectItem key={f.key} value={f.key} text={`${f.key} (${f.base_url})`} />
        ))}
      </Select>
      <TextInput id="source-repository" labelText="Repository" helperText="group/project on GitLab, owner/repo on GitHub" value={repository} onChange={(e) => setRepository(e.target.value)} />
      <TextInput id="source-ref" labelText="Ref" helperText="A branch, a tag or a commit" value={reference} onChange={(e) => setReference(e.target.value)} />
      <TextInput id="source-path" labelText="Path" helperText="Where its items are in the repository; empty for the whole of it" value={path} onChange={(e) => setPath(e.target.value)} />
      <Toggle id="source-live" labelText="Its new versions go live" labelA="Wait for approval" labelB="Go live" toggled={live} onToggle={setLive} />
      <div className={styles.actions}>
        <Button type="submit" size="md" disabled={busy || !isKey(key) || !chosen || repository.trim() === '' || reference.trim() === ''}>
          Add the source
        </Button>
        <Button kind="ghost" size="md" onClick={onDone}>
          Cancel
        </Button>
      </div>
      <Outcome busy={busy} error={refused} refusedTitle="Not added" />
    </Form>
  );
}

// ---------- publish tokens ----------

/// The tokens, never their values (ST-C113); making one, its value shown once (ST-C114).
function Tokens() {
  const client = useQueryClient();
  const tokens = useRead({ queryKey: settingKeys.tokens, queryFn: ({ signal }) => readTokens(signal), retry: false, staleTime: 0 });
  const [making, setMaking] = useState(false);
  const [made, setMade] = useState<{ name: string; token: string } | null>(null);
  const [revoking, setRevoking] = useState<TokenRow | null>(null);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const reread = () => client.invalidateQueries({ queryKey: settingKeys.tokens });
  const revoke = async () => {
    if (!revoking) return;
    setBusy(true);
    setRefused(null);
    try {
      await revokeToken(revoking.id);
      setRevoking(null);
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await reread();
    }
  };
  return (
    <Page.Section
      title="Publish tokens"
      action={
        <Button kind="tertiary" size="sm" onClick={() => setMaking(true)}>
          Make a token
        </Button>
      }
    >
      <Explain>
        For a pipeline or a cloud session that publishes to the Library: a token can publish and do nothing else, and never holds the router’s password.
      </Explain>
      {making && (
        <MakeToken
          onMade={(m) => {
            setMade(m);
            setMaking(false);
          }}
          onCancel={() => setMaking(false)}
          reread={reread}
        />
      )}
      {made && (
        <div className={styles.secretBox} data-token="">
          <p className={styles.lead}>
            The token {made.name}, shown now and never again: keep it where the pipeline reads it, as its bearer token.
          </p>
          <Facts label="The token">
            <Fact term="Token" mono>
              {made.token}
            </Fact>
          </Facts>
          <div className={styles.actions}>
            <CopyButton text={made.token} what="the token" />
          </div>
        </div>
      )}
      {tokens.value === null ? (
        tokens.unreachable ? <ReadFailed what="Could not read the tokens" error={tokens.error} /> : <Reading what="Reading the tokens…" />
      ) : tokens.value.length === 0 ? (
        <p className={styles.lead}>No token yet.</p>
      ) : (
        <ul className={styles.cards}>
          {tokens.value.map((t) => (
            <li key={t.id} className={styles.card} data-token-row={t.name}>
              <Facts label={`The token ${t.name}`}>
                <Fact term="Token">{t.name}</Fact>
                <Fact term="May publish">{scopeWords(t)}</Fact>
                <Fact term="What it publishes">{t.live ? 'goes live' : 'waits for approval'}</Fact>
                <Fact term="Made">{when(t.created_at)}</Fact>
                <Fact term="Last used">{t.last_used_at ? when(t.last_used_at) : 'never'}</Fact>
                {t.revoked_at !== null && <Fact term="Revoked">{when(t.revoked_at)}</Fact>}
              </Facts>
              {t.revoked_at === null && (
                <div className={styles.actions}>
                  <Button kind="danger--ghost" size="sm" onClick={() => setRevoking(t)}>
                    Revoke
                  </Button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      <Confirm
        open={revoking !== null}
        title={`Revoke the token ${revoking?.name ?? ''}?`}
        confirmText="Revoke"
        busy={busy}
        error={refused}
        onConfirm={revoke}
        onClose={() => {
          setRevoking(null);
          setRefused(null);
        }}
      >
        <p>It publishes nothing from now on. What it published stays, and still names it.</p>
      </Confirm>
    </Page.Section>
  );
}

function MakeToken({ onMade, onCancel, reread }: { onMade: (m: { name: string; token: string }) => void; onCancel: () => void; reread: () => Promise<void> }) {
  const [name, setName] = useState('');
  const [kinds, setKinds] = useState<string[]>([]);
  const [items, setItems] = useState('');
  const [live, setLive] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const make = async () => {
    setBusy(true);
    setRefused(null);
    try {
      const named = items
        .split(',')
        .map((i) => i.trim())
        .filter(Boolean);
      const made = await createToken({ name: name.trim(), kinds, items: named, live });
      onMade({ name: made.name, token: made.token });
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await reread();
    }
  };
  return (
    <Form
      className={styles.form}
      aria-label="Make a token"
      onSubmit={(e) => {
        e.preventDefault();
        void make();
      }}
    >
      <TextInput id="token-name" labelText="Name" helperText="Versions it publishes are signed with it" value={name} onChange={(e) => setName(e.target.value)} />
      <fieldset className={styles.fields}>
        <legend className={styles.controlLabel}>Kinds it may publish (none: any)</legend>
        {KINDS.map((k) => (
          <Checkbox
            key={k}
            id={`token-kind-${k}`}
            labelText={k}
            checked={kinds.includes(k)}
            onChange={(_, { checked }) => setKinds(checked ? [...kinds, k] : kinds.filter((x) => x !== k))}
          />
        ))}
      </fieldset>
      <TextInput id="token-items" labelText="Only these items" helperText="kind/name, separated by commas, as skill/bench; empty for any" value={items} onChange={(e) => setItems(e.target.value)} />
      <Toggle id="token-live" labelText="What it publishes goes live" labelA="Waits for approval" labelB="Goes live" toggled={live} onToggle={setLive} />
      <div className={styles.actions}>
        <Button type="submit" size="md" disabled={busy || name.trim() === ''}>
          Make the token
        </Button>
        <Button kind="ghost" size="md" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      <Outcome busy={busy} error={refused} refusedTitle="Not made" />
    </Form>
  );
}

// ---------- machine publishes ----------

/// Whether publishes from connected machines go live: one switch, waiting for approval by default
/// (ST-C115).
function MachinePublishes() {
  const client = useQueryClient();
  const setting = useRead({ queryKey: settingKeys.settings, queryFn: ({ signal }) => readLibrarySettings(signal), retry: false, staleTime: 0 });
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const [done, setDone] = useState<string | null>(null);
  const flip = async (live: boolean) => {
    setBusy(true);
    setRefused(null);
    setDone(null);
    try {
      await setMachinePublishes(live);
      setDone(live ? 'Publishes from machines go live from now on.' : 'Publishes from machines wait for approval from now on.');
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await client.invalidateQueries({ queryKey: settingKeys.settings });
    }
  };
  return (
    <Page.Section title="Publishes from machines">
      <Explain>
        What an agent publishes with <code>agent-router library publish</code>, signed with its machine and its session.
      </Explain>
      {setting.value === null ? (
        setting.unreachable ? <ReadFailed what="Could not read the setting" error={setting.error} /> : <Reading what="Reading the setting…" />
      ) : (
        <Toggle
          id="machine-publishes"
          labelText="Publishes from machines go live"
          labelA="Wait for approval"
          labelB="Go live"
          toggled={setting.value.machine_publishes === 'live'}
          disabled={busy}
          onToggle={(live) => void flip(live)}
        />
      )}
      <Outcome busy={busy} error={refused} done={done} />
    </Page.Section>
  );
}
