// New for agent-router: Settings → Forges, where the Library imports from (docs/library.md, "Forges and
// sources"; ST-C104 to ST-C108, ST-C116). The page follows Northstar's Settings → Forges
// (ui/src/screens/Settings/Forges.tsx): a forge is a key, a kind and an address, with a token that is
// kept on the router and never shown again; a check says in words what is wrong.
import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Button, Checkbox, Form, PasswordInput, Select, SelectItem, TextInput } from '@carbon/react';
import { Page } from '~/components/Page';
import { useRead } from '~/data/held';
import { checkForge, deleteForge, putForge, readForges, settingKeys } from '~/data/library';
import { CHECKED, FORGE_KINDS, isKey, kindLabel, type ForgeKind, type ForgeRow } from '~/model/forges';
import { Confirm, Explain, Fact, Facts, Outcome, ReadFailed, Reading } from './parts';
import styles from './Settings.module.scss';

export default function ForgesPage() {
  const client = useQueryClient();
  const forges = useRead({ queryKey: settingKeys.forges, queryFn: ({ signal }) => readForges(signal), retry: false, staleTime: 0 });
  const [editing, setEditing] = useState<ForgeRow | 'new' | null>(null);
  const [removing, setRemoving] = useState<ForgeRow | null>(null);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const reread = () => client.invalidateQueries({ queryKey: ['library'] });
  const remove = async () => {
    if (!removing) return;
    setBusy(true);
    setRefused(null);
    try {
      await deleteForge(removing.key);
      setRemoving(null);
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await reread();
    }
  };
  return (
    <>
      <Page.Section
        title="Forges"
        action={
          <Button kind="tertiary" size="sm" onClick={() => setEditing('new')}>
            Add a forge
          </Button>
        }
      >
        <Explain>
          Where the Library imports skills, commands and subagents from: GitLab, or GitHub. A forge’s token is kept on the router and never shown again; a public
          GitHub repository needs none.
        </Explain>
        {forges.value === null ? (
          forges.unreachable ? <ReadFailed what="Could not read the forges" error={forges.error} /> : <Reading what="Reading the forges…" />
        ) : forges.value.length === 0 ? (
          <p className={styles.lead}>No forge yet: add one to import from its repositories.</p>
        ) : (
          <ul className={styles.cards}>
            {forges.value.map((f) => (
              <ForgeCard key={f.key} forge={f} onEdit={() => setEditing(f)} onRemove={() => setRemoving(f)} />
            ))}
          </ul>
        )}
      </Page.Section>
      {editing && <ForgeForm forge={editing === 'new' ? null : editing} onDone={() => setEditing(null)} reread={reread} />}
      <Confirm
        open={removing !== null}
        title={`Remove the forge ${removing?.key ?? ''}?`}
        confirmText="Remove"
        busy={busy}
        error={refused}
        onConfirm={remove}
        onClose={() => {
          setRemoving(null);
          setRefused(null);
        }}
      >
        <p>Its token goes with it. A forge a source imports from cannot be removed until the source is.</p>
      </Confirm>
    </>
  );
}

/// One forge: its key, kind, address, whether a token is kept and how many sources use it, and a
/// check (ST-C105, ST-C107).
function ForgeCard({ forge, onEdit, onRemove }: { forge: ForgeRow; onEdit: () => void; onRemove: () => void }) {
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState<{ result: string; words: string } | null>(null);
  const [refused, setRefused] = useState<unknown>(null);
  const check = async () => {
    setChecking(true);
    setRefused(null);
    try {
      setChecked(await checkForge(forge.key));
    } catch (err) {
      setRefused(err);
    } finally {
      setChecking(false);
    }
  };
  return (
    <li className={styles.card} data-forge={forge.key}>
      <Facts label={`The forge ${forge.key}`}>
        <Fact term="Forge">{forge.key}</Fact>
        <Fact term="Kind">{kindLabel(forge.kind)}</Fact>
        <Fact term="Address" mono>
          {forge.base_url}
        </Fact>
        <Fact term="Token">{forge.token_present ? 'kept on the router' : 'none'}</Fact>
        <Fact term="Sources">{String(forge.sources)}</Fact>
      </Facts>
      <div className={styles.actions}>
        <Button kind="tertiary" size="sm" disabled={checking} onClick={check}>
          Check
        </Button>
        <Button kind="ghost" size="sm" onClick={onEdit}>
          Change
        </Button>
        <Button kind="danger--ghost" size="sm" onClick={onRemove}>
          Remove
        </Button>
      </div>
      {checked ? (
        <p role="status" className={checked.result === 'ok' ? styles.good : styles.bad} data-checked={checked.result}>
          {CHECKED[checked.result] ?? checked.result}: {checked.words}
        </p>
      ) : (
        <Outcome busy={checking} busyText="Asking the forge…" error={refused} />
      )}
    </li>
  );
}

/// Adding or changing a forge. A token typed replaces the kept one; left empty on a change, the kept
/// one stays; "Remove the token" keeps none (ST-C106).
function ForgeForm({ forge, onDone, reread }: { forge: ForgeRow | null; onDone: () => void; reread: () => Promise<void> }) {
  const [key, setKey] = useState(forge?.key ?? '');
  const [kind, setKind] = useState<ForgeKind>(forge?.kind ?? 'gitlab');
  const [address, setAddress] = useState(forge?.base_url ?? '');
  const [token, setToken] = useState('');
  const [clear, setClear] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<unknown>(null);
  const badKey = key !== '' && !isKey(key);
  const save = async () => {
    setBusy(true);
    setRefused(null);
    try {
      const body: { kind: string; base_url: string; token?: string | null } = { kind, base_url: address.trim() };
      if (clear) body.token = null;
      else if (token.trim()) body.token = token.trim();
      await putForge(key, body);
      onDone();
    } catch (err) {
      setRefused(err);
    } finally {
      setBusy(false);
      await reread();
    }
  };
  return (
    <Page.Section title={forge ? `Change the forge ${forge.key}` : 'Add a forge'}>
      <Form
        className={styles.form}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <TextInput
          id="forge-key"
          labelText="Key"
          helperText="What sources name it by: lowercase letters and digits, with single hyphens between them"
          value={key}
          disabled={forge !== null}
          onChange={(e) => setKey(e.target.value)}
          invalid={badKey}
          invalidText="Lowercase letters and digits, with single hyphens between them"
        />
        <Select id="forge-kind" labelText="Kind" value={kind} onChange={(e) => setKind(e.target.value as ForgeKind)}>
          {FORGE_KINDS.map((k) => (
            <SelectItem key={k.kind} value={k.kind} text={k.label} />
          ))}
        </Select>
        <TextInput
          id="forge-address"
          labelText="Address"
          helperText="https://gitlab.example.com, https://github.com, or a GitHub Enterprise server"
          value={address}
          onChange={(e) => setAddress(e.target.value)}
        />
        <PasswordInput
          id="forge-token"
          labelText="Token"
          helperText={
            forge?.token_present
              ? 'Leave empty to keep the token kept now. GitLab needs read_api; GitHub, read access to contents.'
              : 'GitLab needs read_api; GitHub, read access to contents. A public GitHub repository needs none.'
          }
          value={token}
          disabled={clear}
          onChange={(e) => setToken(e.target.value)}
          autoComplete="off"
        />
        {forge?.token_present && <Checkbox id="forge-clear" labelText="Remove the token" checked={clear} onChange={(_, { checked }) => setClear(checked)} />}
        <div className={styles.actions}>
          <Button type="submit" size="md" disabled={busy || !isKey(key) || address.trim() === ''}>
            Save
          </Button>
          <Button kind="ghost" size="md" onClick={onDone}>
            Cancel
          </Button>
        </div>
        <Outcome busy={busy} error={refused} refusedTitle="Not saved" />
      </Form>
    </Page.Section>
  );
}
