// New for agent-router: the Machines screen at `/machines`, a hosted router's machines
// (docs/web-console-contracts.md, "Machines"). How to connect one comes first, whatever the list
// holds (MA-C2), and under it every machine the router has seen since it last started: online
// first, each with its platform, how long it has been online or offline, and whether it runs the
// router's own build (MA-C3, MA-C4). A local router takes no machines, and the screen says so
// (MA-C7).
import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button, ContainedList, ContainedListItem } from '@carbon/react';
import { keys } from '~/app/query';
import { EmptyState } from '~/components/EmptyState';
import { InstallCommands } from '~/components/InstallCommands';
import { Page } from '~/components/Page';
import { PageHeader } from '~/components/PageHeader';
import { StatusDot } from '~/components/StatusDot';
import { useRead } from '~/data/held';
import { syncNow } from '~/data/library';
import { machineKeys, readMachines } from '~/data/machines';
import { readHealth } from '~/data/router';
import { useAuth } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { POLL_MS, since } from '~/lib/freshness';
import { AGENTLESS, CONNECT, ON_THIS_MACHINE, SEEN, machineItems, tally, type MachineItem } from '~/model/machine';
import { toldWords } from '~/model/library';
import { isHosted } from '~/model/settings';
import styles from './Machines.module.scss';

const TITLE = 'Machines · Agent Router';

/// Words a screen reader hears between an item's parts, which the eye tells apart by their places.
const Said = ({ children }: { children: string }) => <span className={styles.said}>{children}</span>;

/// `/machines`. What kind of router this is decides what is drawn, from `/api/auth`'s `mode`, and
/// until it has answered neither kind is drawn (MA-C1, MA-C7).
export default function Machines() {
  // The screen names itself in the tab's title (SH-C7).
  useEffect(() => {
    document.title = TITLE;
  }, []);
  const auth = useAuth();
  return (
    <Page>
      <PageHeader title="Machines" />
      {isHosted(auth.value) ? (
        <Hosted />
      ) : auth.value !== null ? (
        // A local router, reached by its address: it takes no machines, so nothing is offered to run
        // and nothing is read (MA-C7).
        <p className={styles.local}>{ON_THIS_MACHINE}</p>
      ) : auth.unreachable ? (
        <EmptyState title="Could not read what kind of router this is">{refusalText(auth.error)}</EmptyState>
      ) : (
        <p className={styles.muted}>Asking the router what kind it is…</p>
      )}
    </Page>
  );
}

/// A hosted router's machines: how to connect one, then every machine it has seen.
function Hosted() {
  // Every five seconds on the console's one clock, as the sessions list is, and never retried
  // quietly (MA-C3, SH-C15). Each read draws the list again, so every "online for" is measured
  // anew with it.
  const list = useRead({ queryKey: machineKeys.list, queryFn: ({ signal }) => readMachines(signal), refetchInterval: POLL_MS, retry: false, staleTime: 0 });
  // The router's own build: an observer of the shell's health read, which polls it (MA-C4).
  const health = useQuery({ queryKey: keys.health, queryFn: ({ signal }) => readHealth(signal), retry: false });
  const items = machineItems(list.value ?? [], health.data?.build ?? null, Date.now());
  const failed = list.stale || list.unreachable ? refusalText(list.error) : null;
  return (
    <>
      {/* First, whatever the list holds: a machine is connected while others are listed (MA-C2). */}
      <Page.Section title={CONNECT}>
        <InstallCommands origin={location.origin} />
      </Page.Section>
      <Page.Section title={SEEN}>
        {list.value !== null && failed !== null && (
          // Kept, never blanked, and marked with its age (MA-C6).
          <p className={styles.stale}>
            Showing what was read {since(list.at)}: the last read failed — {failed}
          </p>
        )}
        {list.value === null ? (
          list.unreachable ? (
            // A read that failed is not an empty list (MA-C6, SH-C22).
            <EmptyState title="Could not read the machines">{failed}</EmptyState>
          ) : (
            <p className={styles.muted}>Reading the machines…</p>
          )
        ) : items.length === 0 ? (
          // Which empty it is: no machine has connected since the router started (MA-C5).
          <p className={styles.none}>{AGENTLESS}</p>
        ) : (
          <ContainedList kind="on-page" size="lg" className={styles.list} label={tally(items)}>
            {items.map((m) => (
              <Machine key={m.id || m.label} m={m} />
            ))}
          </ContainedList>
        )}
      </Page.Section>
    </>
  );
}

/// One machine at a glance: its dot and its label, its platform, online or offline and for how
/// long, and whether it runs the router's own build, quietly when it does (MA-C3, MA-C4); where its
/// Library stands, and, online, a sync of its Library now (LB-C11, LB-C12).
function Machine({ m }: { m: MachineItem }) {
  const [busy, setBusy] = useState(false);
  const [told, setTold] = useState<string | null>(null);
  const sync = async () => {
    setBusy(true);
    try {
      const answer = await syncNow(m.id);
      setTold(answer.machines.map(toldWords).join('; '));
    } catch (err) {
      setTold(refusalText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <ContainedListItem className={styles.machine} data-machine={m.id} data-online={m.online ? '' : undefined}>
      <div className={styles.body}>
        <span className={styles.top}>
          {/* Its words are beside it, so the dot is not said twice (SE-C20). */}
          <StatusDot tone={m.online ? 'busy' : 'closed'} label={m.presence} decorative />
          <span className={styles.name}>{m.label}</span>
          {m.platform && (
            <>
              <Said>, </Said>
              <span className={styles.platform}>{m.platform}</span>
            </>
          )}
        </span>
        <span className={styles.presence}>
          <Said>, </Said>
          {m.presence}
        </span>
        {m.build && (
          <span className={styles.build} data-same={m.build.same ? '' : undefined}>
            <Said>, </Said>
            {m.build.text}
          </span>
        )}
        {m.library && (
          <span className={styles.library} data-library={m.library.needs ? 'needs' : 'well'}>
            <Said>, </Said>
            {m.library.text}
          </span>
        )}
        {m.online && m.library && (
          <span className={styles.sync}>
            <Button kind="ghost" size="sm" disabled={busy} onClick={sync}>
              {busy ? 'Telling it…' : 'Sync its Library now'}
            </Button>
            {told && <span role="status">{told}</span>}
          </span>
        )}
      </div>
    </ContainedListItem>
  );
}
