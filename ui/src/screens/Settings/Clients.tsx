// The Clients page: set up a coding client to route through this router. A hosted router exports
// the configuration, and with a connected machine chosen it reads, applies and restores that
// machine's Claude Code and Codex through its agent (AGEN-0196, AGEN-0200).
import { useId, useRef, useState, type FormEvent } from 'react';
import { Button, CodeSnippet, Form, PasswordInput, Select, SelectItem, Tag, TextInput, Tile } from '@carbon/react';
import { Link } from 'react-router';
import { CopyButton } from '~/components/CopyButton';
import { Page } from '~/components/Page';
import { applyOnMachine, checkClient, clientSetupKey, previewClient, restoreClient, useClientSetup } from '~/data/client-setup';
import { useRead } from '~/data/held';
import { machineKeys, readMachines } from '~/data/machines';
import { useReread } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { POLL_MS, since } from '~/lib/freshness';
import { settingsPath } from '~/lib/routes';
import { clientFilename, clientLimits, type ClientCheck, type ClientPreview, type ClientSetup, type SetupClient, type SetupClientId } from '~/model/client-setup';
import type { MachineRow } from '~/model/machine';
import { providerName } from '~/model/settings';
import { Confirm, Explain, Fact, Facts, Outcome, ReadFailed, Reading } from './parts';
import styles from './Settings.module.scss';

export default function ClientsPage() {
  const setup = useClientSetup();
  const reread = useReread();
  const [selected, setSelected] = useState<SetupClientId>('claude-code');
  // A hosted router's machine choice: none exports the configuration, as before.
  const [machinePick, setMachinePick] = useState('');
  const hosted = setup.value?.mode === 'server';
  const machinesRead = useRead({ queryKey: machineKeys.list, queryFn: ({ signal }) => readMachines(signal), enabled: hosted, refetchInterval: POLL_MS, retry: false, staleTime: 0 });
  const machines = machinesRead.value ?? [];
  const machine = hosted && machinePick ? machinePick : null;
  const there = useClientSetup(machine);
  if (!setup.value) return setup.unreachable ? <ReadFailed what="Could not read client setup" error={setup.error} /> : <Reading what="Reading client setup…" />;
  const label = machineLabel(machines, machine);
  // A machine's read answers for that machine alone; one kept from another choice is not it.
  const shown = machine ? (there.value?.machine === machine ? there.value : null) : setup.value;
  const client = shown ? (shown.clients.find((c) => c.id === selected) ?? shown.clients[0]) : undefined;
  return (
    <>
      <Page.Section title="Clients" action={<Button kind="tertiary" size="sm" onClick={() => void reread(clientSetupKey)}>Refresh status</Button>}>
        <Explain>Choose the coding client you use and a model from any connected provider. Agent Router translates its requests and keeps provider credentials on the router.</Explain>
        {setup.stale ? <p className={styles.subtle}>Showing what was read {since(setup.at)}; the last read failed: {refusalText(setup.error)}</p> : null}
        {hosted ? <Explain>This is a hosted router. Choose a connected machine to set up its Claude Code and Codex from here, or export the configuration and install it on the machine running your client.</Explain> : null}
        {hosted ? <MachineChoice machines={machines} read={machinesRead} value={machinePick} onChange={setMachinePick} /> : null}
        {machine && !shown ? (there.error ? <ReadFailed what={`Could not read ${label}’s clients`} error={there.error} /> : <Reading what={`Reading ${label}’s clients…`} />) : null}
        {machine && shown && there.stale ? <p className={styles.subtle}>Showing what {label} said {since(there.at)}; the last read failed: {refusalText(there.error)}</p> : null}
        {shown ? <div className={styles.cards}>
          {shown.clients.map((c) => <ClientCard key={c.id} client={c} hosted={hosted} machine={machine ? label : null} selected={c.id === client?.id} onSelect={() => setSelected(c.id)} />)}
        </div> : null}
        {setup.value.mode === 'local' ? <Explain><Link to={settingsPath('claude-code')}>Claude Code service, project routing and editor controls</Link> remain available.</Explain> : null}
      </Page.Section>
      {shown && client ? <ConfigureClient key={`${client.id}:${machine ?? ''}`} client={client} setup={shown} machine={machine && client.can_apply ? { id: machine, label } : null} /> : shown ? <Explain>No supported clients were reported by this router.</Explain> : null}
    </>
  );
}

/// A machine as the page names it: its label, else its id.
const machineLabel = (machines: readonly MachineRow[], id: string | null): string => machines.find((m) => m.id === id)?.label || id || '';

/// Which machine the page is about: none, to export, else a connected one. A machine that is not
/// connected now is listed, and cannot be chosen.
function MachineChoice({ machines, read, value, onChange }: { machines: readonly MachineRow[]; read: { readonly stale: boolean; readonly unreachable: boolean; readonly error: unknown }; value: string; onChange: (id: string) => void }) {
  const id = useId();
  const chosen = machines.find((m) => m.id === value);
  return (
    <>
      <Select id={`${id}-machine`} labelText="Machine" value={value} onChange={(e) => onChange(e.target.value)}
        helperText={chosen && !chosen.online ? `${chosen.label} is offline: its clients can be set up once its agent connects again.` : 'Its agent reads, applies and restores Claude Code’s and Codex’s settings there. OpenCode is exported.'}>
        <SelectItem value="" text="None — export the configuration" />
        {machines.map((m) => <SelectItem key={m.id} value={m.id} text={`${m.label} · ${m.platform}${m.online ? '' : ' · offline'}`} disabled={!m.online && m.id !== value} />)}
      </Select>
      {read.unreachable ? <p className={styles.subtle}>Could not read the machines: {refusalText(read.error)}</p> : null}
    </>
  );
}

function ClientCard({ client, hosted, machine, selected, onSelect }: { client: SetupClient; hosted: boolean; machine: string | null; selected: boolean; onSelect: () => void }) {
  return (
    <Tile className={styles.card}>
      <div className={styles.cardHead}><h3 className={styles.cardTitle}>{client.name}</h3><Tag type={client.managed ? 'green' : 'gray'}>{client.managed ? 'Routed' : client.configured ? 'Configuration found' : 'Not configured'}</Tag></div>
      <Facts label={`${client.name} installation`}>
        <Fact term="Installation">{hosted && !(machine && client.can_apply) ? 'Check on your client’s machine' : client.installed ? client.version || 'Installed' : `Not detected on ${machine ?? 'this machine'}`}</Fact>
        {client.config_path ? <Fact term="Configuration" mono>{client.config_path}</Fact> : null}
      </Facts>
      <Button kind={selected ? 'secondary' : 'tertiary'} size="md" onClick={onSelect} aria-pressed={selected}>Configure {client.name}</Button>
    </Tile>
  );
}

/// One client's setup form. With `machine`, a hosted router's chosen machine whose agent reads and
/// writes this client's settings, every call names that machine.
function ConfigureClient({ client, setup, machine }: { client: SetupClient; setup: ClientSetup; machine: { readonly id: string; readonly label: string } | null }) {
  const id = useId();
  const reread = useReread();
  const [model, setModel] = useState(setup.models.find((m) => m.available)?.id ?? '');
  const [url, setUrl] = useState(setup.url);
  const [token, setToken] = useState('');
  const [project, setProject] = useState('');
  // Tokens and generated files stay in component state only. Navigating away or switching
  // client unmounts this form, clearing them; there is deliberately no mutation cache here.
  const [preview, setPreview] = useState<ClientPreview | null>(null);
  const [checks, setChecks] = useState<readonly ClientCheck[] | null>(null);
  const [checkWarnings, setCheckWarnings] = useState<readonly string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<'apply' | 'restore' | null>(null);
  const operation = useRef(false);
  const selected = setup.models.find((m) => m.id === model);
  const input = { model, url: url.trim(), ...(token ? { token } : {}), ...(project.trim() ? { project: project.trim() } : {}) };
  const ready = !!selected?.available && !!url.trim();
  const local = setup.mode === 'local';
  // Where Apply and Restore write: this router's machine, else the chosen one.
  const writes = local || machine !== null;
  const on = machine?.id ?? null;
  const limits = clientLimits(client.id, selected?.provider, selected?.native_protocol, selected?.capabilities);
  const reset = () => { setPreview(null); setChecks(null); setCheckWarnings([]); setError(null); setDone(null); };
  const run = async (act: 'preview' | 'check' | 'apply' | 'restore') => {
    if (operation.current || (act !== 'restore' && !ready)) return;
    operation.current = true;
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      if (act === 'preview') {
        const reviewed = await previewClient(client.id, input, on);
        setPreview(reviewed);
        setChecks(reviewed.checks);
      } else if (act === 'check') {
        const checked = await checkClient(client.id, input, on);
        setChecks(checked.checks);
        setCheckWarnings(checked.warnings);
      } else if (act === 'apply') {
        if (!preview) return;
        const applied = await applyOnMachine(client.id, { ...input, revision: preview.revision }, on);
        const where = machine ? ` on ${machine.label}` : '';
        setDone(applied.changed ? `Saved ${client.name} configuration${where}. Start a new client session to use it.` : `${client.name}${where} already has this configuration.`);
        setConfirm(null);
        await reread(clientSetupKey);
      } else {
        const restored = await restoreClient(client.id, project.trim() || undefined, on);
        setDone(restored.preserved_edits ? 'Restored unchanged settings. Settings you edited remain in place; review the configuration for any remaining router URL or token.' : restored.changed ? 'Restored the previous configuration.' : 'No router-managed configuration needed restoring.');
        setConfirm(null);
        setPreview(null);
        await reread(clientSetupKey);
      }
    } catch (err) {
      setError(err);
    } finally {
      operation.current = false;
      setBusy(false);
    }
  };
  const submit = (event: FormEvent) => { event.preventDefault(); void run('preview'); };
  const warnings = [...new Set([...limits, ...(preview?.warnings ?? []), ...checkWarnings])];
  return (
    <Page.Section title={`Set up ${client.name}`}>
      <Form aria-label={`Set up ${client.name}`} onSubmit={submit} className={styles.form}>
        <Select id={`${id}-model`} labelText="Model" value={model} onChange={(e) => { setModel(e.target.value); reset(); }} disabled={busy} helperText="The exact model ID is sent to the router. Its provider and account are selected by your model routes.">
          {!model ? <SelectItem value="" text="Choose an available model" /> : null}
          {setup.models.map((m) => <SelectItem key={m.id} value={m.id} text={`${m.display_name} · ${providerName(m.provider)} · ${m.id}${m.available ? '' : ' · Account needed'}`} disabled={!m.available} />)}
        </Select>
        {selected?.service_id ? <Explain>Service: <code>{selected.service_id}</code>. API: {selected.native_protocol === 'responses' ? 'OpenAI Responses' : 'Anthropic Messages'}.</Explain> : null}
        {selected?.capabilities ? <Explain>Configured limits: context {selected.capabilities.context_tokens?.toLocaleString() ?? 'unknown'} tokens; output {selected.capabilities.output_tokens?.toLocaleString() ?? 'unknown'} tokens. Tools {selected.capabilities.tools ? 'allowed' : 'disabled'}, images {selected.capabilities.images ? 'allowed' : 'disabled'}, reasoning {selected.capabilities.reasoning ? 'allowed' : 'disabled'}. Provider entitlement is checked separately.</Explain> : null}
        {setup.models.every((m) => !m.available) ? <Explain>Add a provider credential in <Link to={settingsPath('accounts')}>Accounts</Link> before configuring a client.</Explain> : null}
        <TextInput id={`${id}-url`} labelText="Router URL" value={url} onChange={(e) => { setUrl(e.target.value); reset(); }} autoComplete="off" spellCheck={false} disabled={busy} helperText="Use an address reachable from the machine running the client." />
        <PasswordInput id={`${id}-token`} labelText="Client access token" value={token} onChange={(e) => { setToken(e.target.value); reset(); }} autoComplete="new-password" spellCheck={false} disabled={busy} helperText={local ? 'Use a token with Client permissions. A connection to this router on this machine can leave it blank. Existing router passwords also work.' : machine ? `Needed to apply on ${machine.label}: Apply writes it into ${client.name}’s configuration there. Use a token with Client permissions from this router.` : 'Use a token with Client permissions from this router. Existing router passwords also work.'} />
        <Explain>Create a named <Link to={settingsPath('access-tokens')}>Client access token</Link> for this client. Provider API keys belong in Accounts. A preview without a token contains a placeholder to replace on your client’s machine.</Explain>
        {local && client.id !== 'codex' ? <TextInput id={`${id}-project`} labelText="Project directory (optional)" value={project} onChange={(e) => { setProject(e.target.value); reset(); }} autoComplete="off" spellCheck={false} disabled={busy} helperText="Leave blank for the client’s global configuration; enter a full path to configure one project." /> : null}
        {warnings.length > 0 ? <div className={styles.block} role="note" aria-label={`${client.name} compatibility`}><span className={styles.controlLabel}>Compatibility</span>{warnings.map((warning) => <Explain key={warning}>{warning}</Explain>)}</div> : null}
        <div className={styles.actions}>
          <Button type="submit" size="md" disabled={busy || !ready}>Preview configuration</Button>
          <Button kind="tertiary" size="md" disabled={busy || !ready} onClick={() => void run('check')}>Check connection</Button>
          {writes && client.can_apply && (client.managed || project.trim()) ? <Button kind="danger--tertiary" size="md" disabled={busy} onClick={() => { setError(null); setConfirm('restore'); }}>Restore previous configuration</Button> : null}
        </div>
        <Explain>The connection check verifies the model route, provider credential and router token. It does not run a model request.</Explain>
      </Form>
      {preview ? <div className={styles.block}>
        <h3 className={styles.cardTitle}>Configuration preview</h3>
        {preview.path ? <Explain>Settings to add to <code className={styles.path}>{preview.path}</code>{machine ? ` on ${machine.label}` : ''}. Applying merges them into your existing configuration.</Explain> : <Explain>Merge these settings into {client.name}’s configuration on the machine running the client.</Explain>}
        <CodeSnippet type="multi" wrapText hideCopyButton className={styles.snippet} aria-label={`${client.name} configuration preview`}>{preview.content}</CodeSnippet>
        <Explain>{token ? 'This configuration contains your router access token. Save it as a private client configuration.' : 'If this client runs on another machine, replace the token placeholder with a Client access token.'} Copied or downloaded settings should be merged with any existing configuration.</Explain>
        {preview.catalogue_content ? <Explain>Apply installs the server’s models for the Codex CLI and editor. For an exported configuration, run <code>agent-router codex models refresh</code> on the client machine. Finish active turns before restarting Codex, then resume the same conversation. Running clients keep their current model list.</Explain> : null}
        <div className={styles.actions}>
          <CopyButton text={preview.content} label="Copy configuration" what="the configuration" />
          <Button kind="tertiary" size="md" onClick={() => downloadConfiguration(clientFilename(client.id, preview.format), preview.content)}>Download configuration</Button>
          {preview.catalogue_content ? <Button kind="tertiary" size="md" onClick={() => downloadConfiguration('agent-router-models.json', preview.catalogue_content!)}>Download Codex model catalogue</Button> : null}
          {writes && client.can_apply && preview.can_apply ? <Button kind="secondary" size="md" disabled={busy} onClick={() => { setError(null); setConfirm('apply'); }}>Apply configuration</Button> : null}
          <Button kind="ghost" size="md" disabled={busy} onClick={() => { setPreview(null); setToken(''); }}>Clear preview and token</Button>
        </div>
      </div> : null}
      {checks ? <div className={styles.block} aria-label="Connection check results"><h3 className={styles.cardTitle}>Connection check</h3>{checks.map((check) => <div key={check.id}><span className={check.status === 'pass' ? styles.good : check.status === 'fail' ? styles.bad : styles.warn}>{check.label}: {check.status === 'pass' ? 'Passed' : check.status === 'fail' ? 'Failed' : 'Needs attention'}</span><Explain>{check.detail}</Explain></div>)}</div> : null}
      <Outcome busy={busy} busyText="Checking client setup…" error={confirm ? null : error} done={done} />
      <Confirm open={confirm !== null} title={confirm === 'apply' ? `Apply ${client.name} configuration?` : `Restore ${client.name} configuration?`} confirmText={confirm === 'apply' ? 'Apply configuration' : 'Restore configuration'} busy={busy} error={error} onClose={() => { if (!busy) setConfirm(null); }} onConfirm={() => { if (confirm) void run(confirm); }}>
        {confirm === 'apply' ? <><p>Save the configuration you reviewed to <code className={styles.path}>{preview?.path || 'this client’s configuration file'}</code>{machine ? ` on ${machine.label}` : ''}. {machine ? `Its agent saves the previous settings there` : 'The router saves the previous settings'} so you can restore them.</p>{token ? <p>Your router access token will be stored in this private client configuration.</p> : null}</> : <p>Restore routing settings{machine ? ` on ${machine.label}` : ''} that still match what Agent Router saved. Settings you edited are preserved, including any edited router URL or token; review those settings manually afterward.</p>}
      </Confirm>
    </Page.Section>
  );
}

function downloadConfiguration(filename: string, content: string) {
  const href = URL.createObjectURL(new Blob([content], { type: 'text/plain;charset=utf-8' }));
  const link = document.createElement('a');
  link.href = href;
  link.download = filename;
  try { link.click(); }
  finally { URL.revokeObjectURL(href); }
}
