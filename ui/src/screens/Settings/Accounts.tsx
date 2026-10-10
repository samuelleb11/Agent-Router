// New for agent-router: Settings → Accounts. One card per account with Use, Edit and Remove at
// rest, and Add account in place (assets/index.html, `renderAccounts`; ST-C19 to ST-C52). Every
// credential is checked while this page is open, and only then (ST-C6).
import { useState } from 'react';
import { useIsFetching } from '@tanstack/react-query';
import { Button, InlineLoading } from '@carbon/react';
import { Add } from '@carbon/icons-react';
import { Page } from '~/components/Page';
import { EmptyState } from '~/components/EmptyState';
import { settingsKeys, useAccounts, useAuth } from '~/data/settings';
import { since } from '~/lib/freshness';
import { refusalText } from '~/lib/errors';
import { isHosted } from '~/model/settings';
import { AccountCard } from './AccountCard';
import { AddAccount } from './AddAccount';
import { Outcome, ReadFailed, Reading } from './parts';
import { accountsHeading } from './rules';
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
        <div className={styles.cards}>
          {accounts.map((a) => (
            <AccountCard key={a.id} account={a} accounts={accounts} auth={auth.value} hosted={hosted} />
          ))}
        </div>
      )}
    </Page.Section>
  );
}
