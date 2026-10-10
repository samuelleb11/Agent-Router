// New for agent-router: starting a session from the console (docs/web-console-contracts.md,
// "Starting a session"). Only Kimi Code's are started here: `POST /api/live/start` starts one the
// router hosts, on the machine named on a hosted router or on itself on a local one, and the session
// it answers is opened (SE-C239 to SE-C241). The form is a FormModal, mounted only while open, so the
// machines and the catalogue are read only then.
import { useId, useState, type FormEvent } from 'react';
import { Link as RouterLink } from 'react-router';
import { Button, InlineNotification, Select, SelectItem, TextInput } from '@carbon/react';
import { Add } from '@carbon/icons-react';
import { FormModal } from '~/components/FormModal';
import { useRead } from '~/data/held';
import { postLive } from '~/data/live';
import { machineKeys, readMachines } from '~/data/machines';
import { useModels } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { POLL_MS } from '~/lib/freshness';
import { routes, settingsPath } from '~/lib/routes';
import { startBody, startMachines, startModels } from '~/model/session';
import { failed } from './logic';
import styles from './Sessions.module.scss';

/// The button, over the list, and its form.
export const START = 'Start a session';
export const START_TITLE = 'Start a Kimi Code session';
/// With no machine to start one on, or no model to start it with (SE-C239, SE-C240).
export const NO_MACHINE = 'No machine online has Kimi Code.';
export const NO_MODEL = 'No Kimi Code model is in the catalogue — add a Kimi Code account in Settings.';

/// "Start a session" in the list's header, and its form (SE-C239). `hosted` is what `/api/auth`
/// said the router is, null until it has; `onStarted` opens the session the router started.
export function StartSession({ hosted, onStarted }: { hosted: boolean | null; onStarted: (session: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button kind="ghost" size="md" className={styles.findButton} renderIcon={Add} onClick={() => setOpen(true)}>
        {START}
      </Button>
      {open && (
        <StartForm
          hosted={hosted}
          onClose={() => setOpen(false)}
          onStarted={(id) => {
            setOpen(false);
            onStarted(id);
          }}
        />
      )}
    </>
  );
}

/// The form: the machine on a hosted router, the directory, the model, and Start. What it reads is
/// read only while it is open, and a refusal keeps it open with what was typed, in the router's
/// words (SE-C240, SE-C241).
function StartForm({ hosted, onClose, onStarted }: { hosted: boolean | null; onClose: () => void; onStarted: (session: string) => void }) {
  const id = useId();
  // The machines, on the console's one clock, as the Machines screen reads them, and only from a
  // hosted router; a local one starts the session on itself (SE-C239).
  const machinesRead = useRead({ queryKey: machineKeys.list, queryFn: ({ signal }) => readMachines(signal), enabled: hosted === true, refetchInterval: POLL_MS, retry: false, staleTime: 0 });
  const modelsRead = useModels();
  const machines = startMachines(machinesRead.value ?? []);
  const models = startModels(modelsRead.value ?? []);

  const [machinePick, setMachinePick] = useState('');
  const [cwd, setCwd] = useState('');
  const [modelPick, setModelPick] = useState('');
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);

  // The one chosen, else the first: a list that changes under the form never leaves it pointing at
  // nothing.
  const machine = machines.find((m) => m.id === machinePick)?.id ?? machines[0]?.id ?? '';
  const model = models.find((m) => m.id === modelPick)?.id ?? models[0]?.id ?? '';
  const ready = hosted !== null && (hosted === false || machine !== '') && cwd.trim() !== '' && model !== '';

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!ready || busy || hosted === null) return;
    setBusy(true);
    setRefused(null);
    const r = await postLive<{ session?: unknown }>('/api/live/start', startBody(hosted, machine, cwd, model));
    setBusy(false);
    if (r.ok && typeof r.body.session === 'string' && r.body.session) {
      onStarted(r.body.session);
      return;
    }
    // Refused, the form stays with what was typed and says why, in the router's words (SE-C241).
    setRefused(r.ok ? 'The router started nothing it could name.' : failed('start', r));
  };

  return (
    <FormModal
      open
      title={START_TITLE}
      modalLabel="Live sessions"
      submitText="Start"
      submitDisabled={!ready}
      busy={busy}
      onSubmit={(e) => void submit(e)}
      onClose={onClose}
      focusId={`${id}-cwd`}
    >
      <p className={styles.muted}>The router starts Kimi Code there and hosts the session, so it takes messages, answers and a Stop from here.</p>
      {hosted === null ? (
        <p className={styles.muted}>Asking the router what kind it is…</p>
      ) : hosted ? (
        machinesRead.value === null ? (
          machinesRead.unreachable ? (
            // A read that failed is not a router with no machine (SH-C22).
            <p className={styles.muted}>
              Could not read the machines: {refusalText(machinesRead.error)}
            </p>
          ) : (
            <p className={styles.muted}>Reading the machines…</p>
          )
        ) : machines.length ? (
          <Select id={`${id}-machine`} labelText="Machine" value={machine} onChange={(e) => setMachinePick(e.target.value)}>
            {machines.map((m) => (
              <SelectItem key={m.id} value={m.id} text={m.label} />
            ))}
          </Select>
        ) : (
          <p className={styles.muted}>
            {NO_MACHINE}{' '}
            <RouterLink className="cds--link" to={routes.machines}>
              Open Machines
            </RouterLink>
          </p>
        )
      ) : null}
      <TextInput
        id={`${id}-cwd`}
        labelText="Directory"
        helperText={hosted ? 'Its full path on that machine.' : 'Its full path on this router’s machine.'}
        placeholder="/Users/you/code/project"
        value={cwd}
        onChange={(e) => setCwd(e.target.value)}
      />
      {modelsRead.value === null ? (
        modelsRead.unreachable ? (
          <p className={styles.muted}>
            Could not read the models: {refusalText(modelsRead.error)}
          </p>
        ) : (
          <p className={styles.muted}>Reading the models…</p>
        )
      ) : models.length ? (
        <Select id={`${id}-model`} labelText="Model" value={model} onChange={(e) => setModelPick(e.target.value)}>
          {models.map((m) => (
            <SelectItem key={m.id} value={m.id} text={m.label} />
          ))}
        </Select>
      ) : (
        <p className={styles.muted}>
          {NO_MODEL}{' '}
          <RouterLink className="cds--link" to={settingsPath('accounts')}>
            Open Settings → Accounts
          </RouterLink>
        </p>
      )}
      {refused && <InlineNotification kind="error" lowContrast hideCloseButton title="Not started" subtitle={refused} />}
    </FormModal>
  );
}
