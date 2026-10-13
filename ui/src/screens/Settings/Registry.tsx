import { useId, useState, type FormEvent } from 'react';
import { Button, Checkbox, Select, SelectItem, TextInput } from '@carbon/react';
import { Link } from 'react-router';
import { FormModal } from '~/components/FormModal';
import { Page } from '~/components/Page';
import { clientSetupKey } from '~/data/client-setup';
import type { Read } from '~/data/held';
import { providerRegistryKey, removeModelRoute, removeProviderService, saveModelRoute, saveProviderService } from '~/data/provider-registry';
import { settingsKeys, useReread, type AccountRow } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { settingsPath } from '~/lib/routes';
import { DEFAULT_CAPABILITIES, serviceUrlProblem, tokenLimitProblem, type ProviderRegistry, type RegistryCapabilities, type RegistryModel, type RegistryService, type ServiceInput } from '~/model/provider-registry';
import { ADD_PROVIDERS, providerName, providerOf, type AddProvider } from '~/model/settings';
import { Confirm, Explain, Fact, Facts, Outcome, ReadFailed, Reading } from './parts';
import styles from './Settings.module.scss';

export default function Registry({ registry, accounts, hosted }: { registry: Read<ProviderRegistry>; accounts: readonly AccountRow[] | null; hosted: boolean }) {
  const reread = useReread();
  const [serviceEditor, setServiceEditor] = useState<RegistryService | 'new' | null>(null);
  const [modelEditor, setModelEditor] = useState<RegistryModel | 'new' | null>(null);
  const [removing, setRemoving] = useState<{ kind: 'service' | 'model'; id: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState<string | null>(null);
  const refresh = () => reread(providerRegistryKey, settingsKeys.models, clientSetupKey);
  const remove = async () => {
    if (!removing || busy) return;
    setBusy(true); setError(null); setDone(null);
    try {
      if (removing.kind === 'service') await removeProviderService(removing.id);
      else await removeModelRoute(removing.id);
      setDone(`Removed ${removing.kind === 'service' ? 'service' : 'model route'} ${removing.id}.`);
      setRemoving(null);
      await refresh();
    } catch (err) { setError(err); }
    finally { setBusy(false); }
  };
  const value = registry.value;
  return (
    <>
      <Page.Section title="Model routes" action={<Button kind="tertiary" size="sm" disabled={!value || accounts === null || !value.services.length} onClick={() => setModelEditor('new')}>Add model route</Button>}>
        <Explain>Give a model a client-facing name, an exact upstream model ID, a provider service and an account. Clients select the alias; requests use the model and account saved here.</Explain>
        {registry.stale ? <p className={styles.subtle}>Showing what was read {since(registry.at)}; the last read failed: {refusalText(registry.error)}</p> : null}
        {value === null ? registry.unreachable ? <ReadFailed what="Could not read provider services and model routes" error={registry.error} /> : <Reading what="Reading provider services and model routes…" /> : value.models.length === 0 ? <p className={styles.lead}>No explicit model routes yet. The built-in model assignments above remain available.</p> : <ul className={styles.cards}>
          {value.models.map((model) => {
            const service = value.services.find((s) => s.id === model.service);
            const account = accounts?.find((a) => a.id === model.account_id);
            return <li className={styles.card} key={model.alias} data-route={model.alias}>
              <h3 className={styles.cardTitle}>{model.display_name || model.alias}</h3>
              <Facts label={`Model route ${model.alias}`}>
                <Fact term="Client model" mono>{model.alias}</Fact>
                <Fact term="Upstream model" mono>{model.upstream_model}</Fact>
                <Fact term="Service">{service?.label || model.service}</Fact>
                <Fact term="Account">{account?.label || model.account_id}</Fact>
              </Facts>
              {model.capabilities ? <><CapabilitySummary capabilities={model.capabilities} /><Explain>These model settings further restrict the service; the service’s disabled features and lower token limits still apply.</Explain></> : <Explain>Inherits the service’s configured capabilities.</Explain>}
              <div className={styles.actions}>
                <Button kind="tertiary" size="sm" onClick={() => setModelEditor(model)} disabled={accounts === null}>Edit model route {model.alias}</Button>
                <Button kind="danger--ghost" size="sm" onClick={() => { setError(null); setRemoving({ kind: 'model', id: model.alias }); }}>Remove model route {model.alias}</Button>
              </div>
            </li>;
          })}
        </ul>}
        <Outcome done={done} />
      </Page.Section>
      <Page.Section title="Provider services" action={<Button kind="tertiary" size="sm" disabled={!value} onClick={() => setServiceEditor('new')}>Add provider service</Button>}>
        <Explain>A service defines where its model requests go, the API it speaks and how an account authenticates. Provider credentials are managed in <Link to={settingsPath('accounts')}>Accounts</Link>.</Explain>
        <Explain>Configured capabilities are your settings, not a provider entitlement check. Unknown context and output limits remain unknown until you configure them.</Explain>
        {value ? <ul className={styles.cards}>{value.services.map((service) => <li className={styles.card} key={service.id} data-service={service.id}>
          <h3 className={styles.cardTitle}>{service.label}</h3>
          <Facts label={`Provider service ${service.label}`}>
            <Fact term="Service ID" mono>{service.id}</Fact>
            <Fact term="Provider">{providerName(service.provider)}</Fact>
            <Fact term="Base URL" mono>{service.base_url}</Fact>
            <Fact term="API">{service.protocol === 'responses' ? 'OpenAI Responses' : 'Anthropic Messages'}</Fact>
            <Fact term="Authentication">{service.auth_profile === 'bearer' ? 'Bearer credential' : 'API-key header'}</Fact>
          </Facts>
          <CapabilitySummary capabilities={service.capabilities} />
          {service.builtin ? <Explain>Built-in service. Add a separate service for another endpoint.</Explain> : <div className={styles.actions}>
            <Button kind="tertiary" size="sm" onClick={() => setServiceEditor(service)}>Edit provider service {service.label}</Button>
            <Button kind="danger--ghost" size="sm" disabled={value.models.some((m) => m.service === service.id)} onClick={() => { setError(null); setRemoving({ kind: 'service', id: service.id }); }}>Remove provider service {service.label}</Button>
            {value.models.some((m) => m.service === service.id) ? <Explain>Remove this service’s model routes before removing the service.</Explain> : null}
          </div>}
        </li>)}</ul> : null}
      </Page.Section>
      {value && serviceEditor ? <ServiceEditor initial={serviceEditor === 'new' ? null : serviceEditor} services={value.services} onClose={() => setServiceEditor(null)} onSaved={async () => { setServiceEditor(null); setDone('Saved provider service.'); await refresh(); }} /> : null}
      {value && modelEditor ? <ModelEditor initial={modelEditor === 'new' ? null : modelEditor} registry={value} accounts={accounts ?? []} hosted={hosted} onClose={() => setModelEditor(null)} onSaved={async () => { setModelEditor(null); setDone('Saved model route. Choose its client model name in your client.'); await refresh(); }} /> : null}
      <Confirm open={removing !== null} title={`Remove ${removing?.id ?? ''}?`} confirmText="Remove" busy={busy} error={error} onClose={() => { if (!busy) setRemoving(null); }} onConfirm={() => void remove()}>
        <p>{removing?.kind === 'model' ? 'Clients using this alias will lose its explicit route. A built-in model with the same name returns to its normal routing.' : 'Remove this endpoint from the registry. Provider credentials remain in Accounts.'}</p>
      </Confirm>
    </>
  );
}

function CapabilitySummary({ capabilities }: { capabilities: RegistryCapabilities }) {
  return <Facts label="Configured capabilities">
    <Fact term="Tools">{capabilities.tools ? 'Allowed' : 'Disabled'}</Fact>
    <Fact term="Images">{capabilities.images ? 'Allowed' : 'Disabled'}</Fact>
    <Fact term="Reasoning">{capabilities.reasoning ? 'Allowed' : 'Disabled'}</Fact>
    <Fact term="Compaction">{capabilities.compaction ? 'Allowed on native Responses routes' : 'Disabled'}</Fact>
    <Fact term="Context limit">{capabilities.context_tokens?.toLocaleString() ?? 'Unknown'}</Fact>
    <Fact term="Output limit">{capabilities.output_tokens?.toLocaleString() ?? 'Unknown'}</Fact>
  </Facts>;
}

interface CapabilityFields extends Omit<RegistryCapabilities, 'context_tokens' | 'output_tokens'> { context: string; output: string }
const fieldsFor = (capabilities: RegistryCapabilities = DEFAULT_CAPABILITIES): CapabilityFields => ({ ...capabilities, context: capabilities.context_tokens?.toString() ?? '', output: capabilities.output_tokens?.toString() ?? '' });
const configuredCapabilities = (value: CapabilityFields): RegistryCapabilities => ({ tools: value.tools, images: value.images, reasoning: value.reasoning, compaction: value.compaction, context_tokens: value.context.trim() ? Number(value.context) : null, output_tokens: value.output.trim() ? Number(value.output) : null });
const validCapabilities = (value: CapabilityFields) => !tokenLimitProblem(value.context) && !tokenLimitProblem(value.output);

function CapabilitiesEditor({ value, onChange, responses, disabled }: { value: CapabilityFields; onChange: (value: CapabilityFields) => void; responses: boolean; disabled: boolean }) {
  const id = useId();
  return <div className={styles.block}>
    <span className={styles.controlLabel}>Configured capabilities</span>
    {(['tools', 'images', 'reasoning', 'compaction'] as const).map((key) => <Checkbox key={key} id={`${id}-${key}`} labelText={`Allow ${key}`} checked={value[key]} disabled={disabled || (key === 'compaction' && !responses)} onChange={(_, { checked }) => onChange({ ...value, [key]: checked })} />)}
    <Explain>Model settings can further restrict a service. They cannot enable a feature disabled by its service, or increase a service’s token limits. Compaction requires a native Responses route.</Explain>
    <div className={styles.fields}>
      <TextInput id={`${id}-context`} labelText="Context token limit (optional)" value={value.context} inputMode="numeric" disabled={disabled} onChange={(e) => onChange({ ...value, context: e.target.value })} helperText="Leave blank if unknown." invalid={!!tokenLimitProblem(value.context)} invalidText={tokenLimitProblem(value.context) ?? undefined} />
      <TextInput id={`${id}-output`} labelText="Output token limit (optional)" value={value.output} inputMode="numeric" disabled={disabled} onChange={(e) => onChange({ ...value, output: e.target.value })} helperText="Leave blank if unknown." invalid={!!tokenLimitProblem(value.output)} invalidText={tokenLimitProblem(value.output) ?? undefined} />
    </div>
  </div>;
}

function ServiceEditor({ initial, services, onClose, onSaved }: { initial: RegistryService | null; services: readonly RegistryService[]; onClose: () => void; onSaved: () => Promise<void> }) {
  const [id, setId] = useState(initial?.id ?? '');
  const [label, setLabel] = useState(initial?.label ?? '');
  const [provider, setProvider] = useState<AddProvider>(initial?.provider ?? 'openai');
  const [url, setUrl] = useState(initial?.base_url ?? '');
  const [protocol, setProtocol] = useState<ServiceInput['protocol']>(initial?.protocol ?? 'responses');
  const [profile, setProfile] = useState<ServiceInput['auth_profile']>(initial?.auth_profile ?? 'bearer');
  const [capabilities, setCapabilities] = useState(fieldsFor(initial?.capabilities));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const duplicate = !initial && services.some((s) => s.id === id);
  const validId = /^[a-z0-9_-]{1,64}$/.test(id);
  const ready = validId && !duplicate && !!label.trim() && !serviceUrlProblem(url.trim()) && validCapabilities(capabilities);
  const submit = async (event: FormEvent) => {
    event.preventDefault(); if (busy || !ready) return;
    setBusy(true); setError(null);
    try { await saveProviderService({ id, label: label.trim(), provider, base_url: url.trim(), protocol, auth_profile: profile, capabilities: configuredCapabilities(capabilities) }); await onSaved(); }
    catch (err) { setError(err); }
    finally { setBusy(false); }
  };
  return <FormModal open title={initial ? 'Edit provider service' : 'Add provider service'} submitText="Save provider service" submitDisabled={!ready} busy={busy} onClose={() => { if (!busy) onClose(); }} onSubmit={(e) => void submit(e)} focusId="provider-service-id" size="md">
    <TextInput id="provider-service-id" labelText="Service ID" value={id} onChange={(e) => setId(e.target.value)} disabled={busy || !!initial} helperText="1–64 lowercase letters, digits, underscores or hyphens." invalid={!!id && (!validId || duplicate)} invalidText={duplicate ? 'This service already exists. Edit it from its card.' : 'Use lowercase letters, digits, underscores or hyphens.'} />
    <TextInput id="provider-service-label" labelText="Service name" value={label} onChange={(e) => setLabel(e.target.value)} disabled={busy} maxLength={100} />
    <Select id="provider-service-provider" labelText="Account provider" value={provider} onChange={(e) => setProvider(e.target.value as AddProvider)} disabled={busy} helperText="This service uses credentials from accounts belonging to this provider.">{ADD_PROVIDERS.map((p) => <SelectItem key={p} value={p} text={providerName(p)} />)}</Select>
    <TextInput id="provider-service-url" labelText="Service base URL" value={url} onChange={(e) => setUrl(e.target.value)} disabled={busy} autoComplete="off" spellCheck={false} placeholder="https://provider.example" invalid={!!url && !!serviceUrlProblem(url.trim())} invalidText={serviceUrlProblem(url.trim()) ?? undefined} helperText="The HTTPS origin or API prefix, without a final /v1." />
    <Select id="provider-service-protocol" labelText="API protocol" value={protocol} onChange={(e) => { const next = e.target.value as ServiceInput['protocol']; setProtocol(next); if (next === 'messages') setCapabilities({ ...capabilities, compaction: false }); }} disabled={busy}><SelectItem value="responses" text="OpenAI Responses" /><SelectItem value="messages" text="Anthropic Messages" /></Select>
    <Select id="provider-service-auth" labelText="Authentication header" value={profile} onChange={(e) => setProfile(e.target.value as ServiceInput['auth_profile'])} disabled={busy}><SelectItem value="bearer" text="Authorization: Bearer" /><SelectItem value="api_key" text="x-api-key" /></Select>
    <CapabilitiesEditor value={capabilities} onChange={setCapabilities} responses={protocol === 'responses'} disabled={busy} />
    <Explain>Custom services need API-key accounts. Native subscription logins stay with their built-in service. No credentials are entered in this form.</Explain>
    <Outcome busy={busy} error={error} />
  </FormModal>;
}

function ModelEditor({ initial, registry, accounts, hosted, onClose, onSaved }: { initial: RegistryModel | null; registry: ProviderRegistry; accounts: readonly AccountRow[]; hosted: boolean; onClose: () => void; onSaved: () => Promise<void> }) {
  const [alias, setAlias] = useState(initial?.alias ?? '');
  const [upstream, setUpstream] = useState(initial?.upstream_model ?? '');
  const [displayName, setDisplayName] = useState(initial?.display_name ?? '');
  const [serviceId, setServiceId] = useState(initial?.service ?? registry.services[0]?.id ?? '');
  const [accountId, setAccountId] = useState(initial?.account_id ?? '');
  const [overrides, setOverrides] = useState(!!initial?.capabilities);
  const [capabilities, setCapabilities] = useState(fieldsFor(initial?.capabilities ?? undefined));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const service = registry.services.find((s) => s.id === serviceId);
  const eligible = accounts.filter((a) => providerOf(a) === service?.provider && ((service?.builtin && (!hosted || a.source === 'chatgpt')) || a.source === 'token'));
  const nameValid = (value: string) => /^[a-zA-Z0-9_.:/-]{1,200}$/.test(value);
  const duplicate = !initial && registry.models.some((m) => m.alias === alias);
  const ready = !!service && nameValid(alias) && !duplicate && nameValid(upstream) && eligible.some((a) => a.id === accountId) && (!overrides || validCapabilities(capabilities));
  const submit = async (event: FormEvent) => {
    event.preventDefault(); if (busy || !ready) return;
    setBusy(true); setError(null);
    try { await saveModelRoute({ alias, service: serviceId, upstream_model: upstream, account_id: accountId, display_name: displayName.trim() || null, capabilities: overrides ? configuredCapabilities(capabilities) : null }); await onSaved(); }
    catch (err) { setError(err); }
    finally { setBusy(false); }
  };
  return <FormModal open title={initial ? 'Edit model route' : 'Add model route'} submitText="Save model route" submitDisabled={!ready} busy={busy} onClose={() => { if (!busy) onClose(); }} onSubmit={(e) => void submit(e)} focusId="registry-model-alias" size="md">
    <TextInput id="registry-model-alias" labelText="Client model name" value={alias} onChange={(e) => setAlias(e.target.value)} disabled={busy || !!initial} autoComplete="off" spellCheck={false} helperText="The exact alias clients select. Reusing a built-in model name overrides its usual route." invalid={!!alias && (!nameValid(alias) || duplicate)} invalidText={duplicate ? 'This route already exists. Edit it from its card.' : 'Use up to 200 letters, digits, underscores, hyphens, dots, slashes or colons.'} />
    <TextInput id="registry-model-display" labelText="Display name (optional)" value={displayName} onChange={(e) => setDisplayName(e.target.value)} disabled={busy} maxLength={100} />
    <Select id="registry-model-service" labelText="Provider service" value={serviceId} disabled={busy} onChange={(e) => { setServiceId(e.target.value); setAccountId(''); if (registry.services.find((s) => s.id === e.target.value)?.protocol === 'messages') setCapabilities({ ...capabilities, compaction: false }); }}>{registry.services.map((s) => <SelectItem key={s.id} value={s.id} text={`${s.label} · ${s.id}`} />)}</Select>
    <TextInput id="registry-model-upstream" labelText="Upstream model ID" value={upstream} onChange={(e) => setUpstream(e.target.value)} disabled={busy} autoComplete="off" spellCheck={false} helperText="The exact model name expected by the selected service." invalid={!!upstream && !nameValid(upstream)} invalidText="Use up to 200 letters, digits, underscores, hyphens, dots, slashes or colons." />
    <Select id="registry-model-account" labelText="Provider account" value={accountId} onChange={(e) => setAccountId(e.target.value)} disabled={busy} helperText="The account whose credential signs requests to this service."><SelectItem value="" text="Choose an account" />{eligible.map((a) => <SelectItem key={a.id} value={a.id} text={`${a.label || a.email || a.id} · ${a.id}`} />)}</Select>
    {eligible.length === 0 ? <Explain>Add a compatible API-key account in Accounts before saving this route.</Explain> : null}
    {service && !service.builtin ? <Explain>Custom services use API-key accounts. Subscription tokens and native client logins cannot be sent to another endpoint.</Explain> : null}
    <Checkbox id="registry-model-overrides" labelText="Override configured capabilities" checked={overrides} disabled={busy} onChange={(_, { checked }) => { setOverrides(checked); if (checked && !initial?.capabilities) setCapabilities(fieldsFor(service?.capabilities)); }} />
    {overrides ? <CapabilitiesEditor value={capabilities} onChange={setCapabilities} responses={service?.protocol === 'responses'} disabled={busy} /> : <Explain>Inherit the service’s configured capabilities and limits.</Explain>}
    <Outcome busy={busy} error={error} />
  </FormModal>;
}
