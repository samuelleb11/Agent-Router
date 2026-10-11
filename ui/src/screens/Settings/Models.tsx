// New for agent-router: Settings → Models, the catalogue (docs/sources-and-models.md, "The catalogue";
// ST-C122, ST-C123). Every model the router can serve, each with its provider and the account its next
// request goes to, and a list box to give it an account of its own provider or back to its provider's
// active account. A client picks an account by picking a model, so this page is where that is decided.
import { useId, useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Dropdown } from '@carbon/react';
import { Page } from '~/components/Page';
import { useProviderRegistry } from '~/data/provider-registry';
import { assignModel, settingsKeys, useAccounts, useAuth, useModels, useReread, type AccountRow, type ModelRow } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { isHosted, providerName, providerOf } from '~/model/settings';
import { Explain, Fact, Facts, Outcome, ReadFailed, Reading } from './parts';
import { assignedWords, modelChoices, modelName, servedBy, type ModelChoice } from './rules';
import styles from './Settings.module.scss';
import Registry from './Registry';

export default function ModelsPage() {
  const auth = useAuth();
  const models = useModels();
  const registry = useProviderRegistry();
  // The accounts each list box offers. Read without a probe: nothing here needs a credential checked
  // (ST-C6).
  const accounts = useAccounts({ probe: false });
  const hosted = isHosted(auth.value);
  return (
    <>
    <Page.Section title="Models">
      <Explain>
        Every model the router can serve, and the account its requests go to. Choose a model in Claude Code or Codex to use its provider’s account.
        A model given no account of its own goes to its provider’s active account.
      </Explain>
      {models.stale ? (
        <p className={styles.subtle}>
          Showing what was read {since(models.at)}; the last read failed: {refusalText(models.error)}
        </p>
      ) : null}
      {accounts.unreachable ? <ReadFailed what="Could not read the accounts, so no model can be given one" error={accounts.error} /> : null}
      {models.value === null ? (
        models.unreachable ? (
          <ReadFailed what="Could not read the models" error={models.error} />
        ) : (
          <Reading what="Reading the models…" />
        )
      ) : models.value.length === 0 ? (
        <p className={styles.lead}>No model yet. Add a provider account or send a request through the router to discover its models.</p>
      ) : (
        <ul className={styles.cards}>
          {models.value.filter((m) => !registry.value?.models.some((route) => route.alias === m.id)).map((m) => (
            <ModelCard key={m.id} model={m} accounts={accounts.value} hosted={hosted} />
          ))}
        </ul>
      )}
    </Page.Section>
    <Registry registry={registry} accounts={accounts.value} hosted={hosted} />
    </>
  );
}

/// One model: its name, id, provider and who serves it, and its list box (ST-C122, ST-C123). The box
/// shows the assignment the router reports, read back after a write, never one chosen in advance
/// (ST-C13); a refusal stays beside it in the router's words (ST-C9, ST-C10).
function ModelCard({ model, accounts, hosted }: { model: ModelRow; accounts: readonly AccountRow[] | null; hosted: boolean }) {
  const id = useId();
  const reread = useReread();
  const [done, setDone] = useState<string | null>(null);
  const choices = modelChoices(model, accounts ?? [], hosted);
  const selected = choices.find((c) => c.id === (model.assigned ?? null)) ?? choices[0];
  const assign = useMutation({
    mutationFn: async (choice: ModelChoice) => {
      try {
        await assignModel(model.id, choice.id);
      } finally {
        await reread(settingsKeys.models);
      }
      return choice;
    },
    onSuccess: (choice) => setDone(assignedWords(model, choice)),
  });
  const name = modelName(model);
  const provider = providerName(providerOf(model));
  return (
    <li className={styles.card} data-model={model.id}>
      <h3 className={styles.cardTitle}>{name}</h3>
      <Facts label={`What the router knows about ${name}`}>
        <Fact term="Id" mono>
          {model.id}
        </Fact>
        <Fact term="Provider">{provider}</Fact>
        <Fact term="Served by">{servedBy(model, accounts ?? [])}</Fact>
      </Facts>
      <Dropdown<ModelChoice>
        id={`${id}-account`}
        titleText="Account"
        label="Choose an account"
        items={choices}
        itemToString={(c) => c?.label ?? ''}
        selectedItem={selected}
        // Until the accounts are read there is nothing to choose from but the active one.
        disabled={assign.isPending || accounts === null}
        helperText={`${provider}’s accounts only: another provider’s key cannot serve this model.`}
        onChange={({ selectedItem }) => {
          if (!selectedItem || selectedItem.id === (model.assigned ?? null)) return;
          setDone(null);
          assign.mutate(selectedItem);
        }}
      />
      <Outcome busy={assign.isPending} busyText="Assigning…" error={assign.error} done={done} />
    </li>
  );
}
