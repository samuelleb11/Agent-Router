// New for agent-router: Settings → Accounts. One section per provider, each with a list box that
// says which of its accounts its requests go to, then one card per account with Edit and Remove at
// rest, and Add account in place (assets/index.html, `renderAccounts`; ST-C19 to ST-C52, ST-C124,
// ST-C125). Every credential is checked while this page is open, and only then (ST-C6).
import { useId, useState } from 'react';
import { Link } from 'react-router';
import { useIsFetching, useMutation } from '@tanstack/react-query';
import { Button, Dropdown, InlineLoading } from '@carbon/react';
import { Add } from '@carbon/icons-react';
import { keys } from '~/app/query';
import { Page } from '~/components/Page';
import { EmptyState } from '~/components/EmptyState';
import { ProviderMark } from '~/components/ProviderMark';
import { activateAccount, settingsKeys, useAccounts, useAuth, useReread, type AccountRow } from '~/data/settings';
import { since } from '~/lib/freshness';
import { refusalText } from '~/lib/errors';
import { settingsPath } from '~/lib/routes';
import { byProvider, isHosted, providerName, type Auth, type Provider } from '~/model/settings';
import { AccountCard } from './AccountCard';
import { AddAccount } from './AddAccount';
import { Outcome, ReadFailed, Reading } from './parts';
import { accountsHeading, activeChoices, flag, type ActiveChoice } from './rules';
import styles from './Settings.module.scss';

export default function AccountsPage() {
  const auth = useAuth();
  const list = useAccounts({ probe: true });
  const checking = useIsFetching({ queryKey: settingsKeys.probed }) > 0;
  const [adding, setAdding] = useState(false);
  const [added, setAdded] = useState<string | null>(null);
  const hosted = isHosted(auth.value);
  const accounts = list.value ?? [];

  return (
    <Page.Section
      title="Accounts"
      action={
        <Button
          kind="primary"
          size="md"
          renderIcon={Add}
          disabled={adding || list.value === null}
          onClick={() => {
            setAdded(null);
            setAdding(true);
          }}
        >
          Add account
        </Button>
      }
    >
      <div className={styles.lead}>
        {list.value !== null ? <span>{accountsHeading(accounts)}</span> : null}
        {checking ? <InlineLoading className={styles.leadLoading} description="Checking each credential…" status="active" /> : null}
      </div>
      {list.stale ? (
        <p className={styles.subtle}>
          Showing what was read {since(list.at)}; the last read failed: {refusalText(list.error)}
        </p>
      ) : null}

      {adding ? (
        <AddAccount
          auth={auth.value}
          accounts={accounts}
          onClose={() => setAdding(false)}
          onDone={(said) => {
            setAdding(false);
            setAdded(said);
          }}
        />
      ) : null}
      <Outcome done={added} />

      {list.unreachable ? (
        <ReadFailed what="Could not read the accounts" error={list.error} />
      ) : list.value === null ? (
        <Reading what="Reading the accounts and checking each credential…" />
      ) : accounts.length === 0 ? (
        <EmptyState title="No accounts yet">Nothing can sign a request until one is added. Add account, above, is the way in.</EmptyState>
      ) : (
        byProvider(accounts).map((g) => <ProviderSection key={g.provider} provider={g.provider} own={g.accounts} accounts={accounts} auth={auth.value} hosted={hosted} />)
      )}
    </Page.Section>
  );
}

/// One provider's accounts, headed by its mark and name (ST-C125, SH-C56): above its cards, a list box
/// that says which account its requests go to, with its active account selected. Picking another
/// makes that one the provider's active account, in one choice with no confirmation, through the call
/// Use made; the list re-reads whatever the router said, the box shows what it reads back, and a
/// refusal stays beside the box in the router's words (ST-C13, ST-C24, ST-C25, ST-C121).
function ProviderSection({ provider, own, accounts, auth, hosted }: { provider: Provider; own: readonly AccountRow[]; accounts: readonly AccountRow[]; auth: Auth | null; hosted: boolean }) {
  const id = useId();
  const reread = useReread();
  const name = providerName(provider);
  const choices = activeChoices(own, hosted);
  const active = own.find((a) => flag(a.active));
  // Null, never undefined, when the provider has no active account: the box stays the router's to
  // fill (ST-C13), showing nothing chosen and asking for one.
  const selected = choices.find((c) => c.id === active?.id) ?? null;
  const use = useMutation({
    mutationFn: async (choice: ActiveChoice) => {
      try {
        await activateAccount(choice.id);
      } finally {
        await reread(keys.accounts);
      }
    },
  });
  return (
    <section className={styles.provider} aria-labelledby={`${id}-heading`}>
      <h3 id={`${id}-heading`} className={styles.providerTitle}>
        <ProviderMark provider={provider} />
      </h3>
      <div className={styles.requestsTo}>
        <Dropdown<ActiveChoice | null>
          id={`${id}-active`}
          titleText="Requests go to"
          label="Choose an account"
          items={choices}
          itemToString={(c) => c?.label ?? ''}
          selectedItem={selected}
          // Nothing to choose from on a hosted router whose accounts it cannot sign with (ST-C27).
          disabled={use.isPending || choices.length === 0}
          helperText={
            <>
              {name}’s models go to this account unless the <Link to={settingsPath('models')}>Models</Link> page assigns a model elsewhere.
            </>
          }
          onChange={({ selectedItem }) => {
            if (!selectedItem || selectedItem.id === active?.id) return;
            use.mutate(selectedItem);
          }}
        />
        <Outcome busy={use.isPending} busyText="Switching…" error={use.error} />
      </div>
      <div className={styles.cards}>
        {own.map((a) => (
          <AccountCard key={a.id} account={a} accounts={accounts} auth={auth} hosted={hosted} />
        ))}
      </div>
    </section>
  );
}
