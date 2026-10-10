// New for agent-router: Settings → Notifications (assets/index.html, `renderPush`; ST-C84 to
// ST-C94). Apple's key, which is never sent back and never pre-filled; Firebase, which the router
// already reports and accepts; the phones; and a test whose answer stays on screen after the page
// reads again.
import { useId, useState, type FormEvent } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, Form, Tag, TextArea, TextInput } from '@carbon/react';
import { Page } from '~/components/Page';
import { clearPushKey, saveFirebase, savePushKey, sendTest, settingsKeys, useAuth, usePush, useReread, type Device, type Push } from '~/data/settings';
import { refusalText } from '~/lib/errors';
import { since } from '~/lib/freshness';
import { notes, pushKeyHint, type Auth } from '~/model/settings';
import { Confirm, Explain, Fact, Facts, Outcome, ReadFailed, Reading, Words } from './parts';
import { deviceRow, NO_PHONES, pushHeading, pushKeyProblem, serviceAccountProblem } from './rules';
import styles from './Settings.module.scss';

export default function NotificationsPage() {
  const auth = useAuth();
  const push = usePush();
  const p = push.value;
  if (!p) return push.unreachable ? <ReadFailed what="Could not read notification settings" error={push.error} /> : <Reading what="Reading notification settings…" />;
  const devices = p.devices ?? [];
  return (
    <>
      <Page.Section title="Notifications">
        <p className={styles.lead}>{pushHeading(!!p.configured, p.key_id, devices.length)}</p>
        {push.stale ? (
          <p className={styles.subtle}>
            Showing what was read {since(push.at)}; the last read failed: {refusalText(push.error)}
          </p>
        ) : null}
        <Explain>
          <Words segments={notes(auth.value).push} />
        </Explain>
        <Test push={p} />
      </Page.Section>
      <Apple push={p} auth={auth.value} />
      <Firebase push={p} />
      <Page.Section title="Phones">
        {devices.length === 0 ? (
          <Explain>{NO_PHONES}</Explain>
        ) : (
          <ul className={styles.rows} aria-label="Registered phones">
            {devices.map((d) => (
              <Phone key={d.token_tail} device={d} />
            ))}
          </ul>
        )}
      </Page.Section>
    </>
  );
}

function Phone({ device }: { device: Device }) {
  const row = deviceRow(device);
  const failed = !!device.last_result && device.last_result !== 'delivered';
  return (
    <li className={styles.row}>
      <span className={styles.rowText}>
        <span className={styles.mono}>{row.who}</span>
        <span className={failed ? styles.bad : styles.subtle}>{row.last}</span>
      </span>
    </li>
  );
}

/// Apple's key: the ids filled from what is stored, the `.p8` never (ST-C87). The three are asked
/// for together before anything is sent (ST-C88), and a refused key stays in its box.
function Apple({ push, auth }: { push: Push; auth: Auth | null }) {
  const reread = useReread();
  const id = useId();
  // Undefined until typed: an untouched field follows what the router has stored.
  const [team, setTeam] = useState<string | undefined>(undefined);
  const [kid, setKid] = useState<string | undefined>(undefined);
  const [bundle, setBundle] = useState<string | undefined>(undefined);
  const [p8, setP8] = useState('');
  const [tried, setTried] = useState(false);
  const [asking, setAsking] = useState(false);
  const configured = !!push.configured;
  const values = { team: team ?? push.team_id ?? '', kid: kid ?? push.key_id ?? '', bundle: bundle ?? push.bundle_id ?? '' };
  const problem = pushKeyProblem(values.team, values.kid, p8);
  const hint = pushKeyHint(push, auth);

  const save = useMutation({
    mutationFn: async () => {
      await savePushKey({ team_id: values.team, key_id: values.kid, p8, bundle_id: values.bundle });
      // Kept by the router, proven to sign: the key leaves the screen.
      setP8('');
      setTeam(undefined);
      setKid(undefined);
      setBundle(undefined);
      setTried(false);
      await reread(settingsKeys.push);
    },
  });
  const clear = useMutation({
    mutationFn: async () => {
      await clearPushKey();
      await reread(settingsKeys.push);
    },
    onSuccess: () => setAsking(false),
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    setTried(true);
    save.reset();
    if (problem || save.isPending) return;
    save.mutate();
  };

  return (
    <Page.Section title="Apple (iPhone)">
      {hint ? <Explain className={styles.mono}>{hint}</Explain> : null}
      <Form aria-label="The APNs key" onSubmit={submit} className={styles.form}>
        <div className={styles.fields3}>
          <TextInput id={`${id}-team`} labelText="Team id" placeholder="ABCDE12345" autoComplete="off" spellCheck={false} value={values.team} onChange={(e) => setTeam(e.target.value)} />
          <TextInput id={`${id}-kid`} labelText="Key id" placeholder="ABC123DEF4" autoComplete="off" spellCheck={false} value={values.kid} onChange={(e) => setKid(e.target.value)} />
          <TextInput id={`${id}-bundle`} labelText="Bundle id" placeholder="the app’s bundle id" autoComplete="off" spellCheck={false} value={values.bundle} onChange={(e) => setBundle(e.target.value)} />
        </div>
        <TextArea
          id={`${id}-p8`}
          labelText="APNs key (.p8)"
          rows={4}
          spellCheck={false}
          autoComplete="off"
          className={styles.secretBox}
          placeholder={configured ? 'a new .p8 replaces the stored one' : '-----BEGIN PRIVATE KEY-----'}
          value={p8}
          onChange={(e) => setP8(e.target.value)}
          helperText={configured ? 'A key is stored, and is never shown here: a new one pasted here replaces it.' : 'Pasted once; the router proves it signs before keeping it.'}
          invalid={tried && !!problem}
          invalidText={problem ?? undefined}
        />
        <div className={styles.actions}>
          <Button type="submit" kind="primary" size="md" disabled={save.isPending}>
            {configured ? 'Replace key' : 'Save key'}
          </Button>
          {configured ? (
            <Button
              kind="danger--tertiary"
              size="md"
              onClick={() => {
                clear.reset();
                setAsking(true);
              }}
            >
              Remove key
            </Button>
          ) : null}
        </div>
        <Outcome
          busy={save.isPending}
          busyText="Checking that the key signs…"
          error={save.error}
          done={save.isSuccess ? 'Saved — the router proved the key signs.' : clear.isSuccess ? 'Removed — phones are not told until a key is stored again.' : null}
        />
      </Form>
      {push.last_error ? <p className={styles.bad}>Last failure: {push.last_error}</p> : null}
      <Confirm
        open={asking}
        title="Remove the APNs key?"
        label="Notifications"
        confirmText="Remove key"
        busy={clear.isPending}
        error={clear.error}
        onClose={() => setAsking(false)}
        onConfirm={() => clear.mutate()}
      >
        <p>Phones stop being told when a session waits on you, until a key is stored again.</p>
        <p>Apple lets a key be downloaded only once: without a copy of the .p8, this one may be gone for good.</p>
      </Confirm>
    </Page.Section>
  );
}

/// Firebase, for the Android app: what the router reports, and a box for the service-account JSON
/// whole, as the Firebase console downloads it (ST-C93).
function Firebase({ push }: { push: Push }) {
  const reread = useReread();
  const id = useId();
  const [json, setJson] = useState('');
  const [tried, setTried] = useState(false);
  const fcm = push.fcm ?? {};
  const problem = serviceAccountProblem(json);
  const save = useMutation({
    mutationFn: async () => {
      await saveFirebase(json);
      setJson('');
      setTried(false);
      await reread(settingsKeys.push);
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    setTried(true);
    save.reset();
    if (problem || save.isPending) return;
    save.mutate();
  };
  return (
    <Page.Section title="Firebase (Android)">
      <Facts label="Firebase">
        <Fact term="Service account">
          <Tag type={fcm.configured ? 'green' : 'gray'} size="sm">
            {fcm.configured ? 'configured' : 'not configured'}
          </Tag>
        </Fact>
        <Fact term="Project" mono>
          {fcm.project_id || '—'}
        </Fact>
        <Fact term="Key file" mono>
          {fcm.key_file || '—'}
        </Fact>
        {fcm.client_email ? (
          <Fact term="Client" mono>
            {fcm.client_email}
          </Fact>
        ) : null}
      </Facts>
      {fcm.last_error ? <p className={styles.bad}>Last failure: {fcm.last_error}</p> : null}
      <Form aria-label="The Firebase service account" onSubmit={submit} className={styles.form}>
        <TextArea
          id={`${id}-sa`}
          labelText="Service-account JSON"
          rows={4}
          spellCheck={false}
          autoComplete="off"
          className={styles.secretBox}
          placeholder={fcm.configured ? 'a new service account replaces the stored one' : '{ "type": "service_account", … }'}
          value={json}
          onChange={(e) => setJson(e.target.value)}
          helperText="The whole file the Firebase console downloads. The router proves its key signs before keeping it."
          invalid={tried && !!problem}
          invalidText={problem ?? undefined}
        />
        <div className={styles.actions}>
          <Button type="submit" kind="primary" size="md" disabled={save.isPending}>
            {fcm.configured ? 'Replace service account' : 'Save service account'}
          </Button>
        </div>
        <Outcome busy={save.isPending} busyText="Checking that the key signs…" error={save.error} done={save.isSuccess ? 'Saved — the router proved the key signs.' : null} />
      </Form>
    </Page.Section>
  );
}

interface Tested {
  readonly none: boolean;
  readonly rows: readonly { readonly device: string; readonly ok: boolean; readonly said: string }[];
}

/// A test push to every phone. Offered whenever either service has a key; its answer, per phone,
/// is kept here, so the page reading again does not take it away (ST-C89).
function Test({ push }: { push: Push }) {
  const reread = useReread();
  const client = useQueryClient();
  const test = useMutation({
    mutationFn: async (): Promise<Tested> => {
      const before = push.devices ?? [];
      const answer = await sendTest();
      await reread(settingsKeys.push);
      if ('results' in answer) {
        return { none: answer.results.length === 0, rows: answer.results.map((r) => ({ device: r.device, ok: r.ok, said: r.ok ? 'delivered' : r.error || 'not delivered' })) };
      }
      // No phone took it, and the router named none in words: each phone's own result, read again.
      const after = client.getQueryData<Push>(settingsKeys.push)?.devices ?? [];
      const rows = before.flatMap((d) => {
        const now = after.find((x) => x.token_tail === d.token_tail);
        if (!now) return [{ device: d.token_tail, ok: false, said: 'no longer registered' }];
        if (now.last_seen_at === d.last_seen_at && now.last_result === d.last_result) return [];
        return [{ device: d.token_tail, ok: now.last_result === 'delivered', said: now.last_result || 'not delivered' }];
      });
      return { none: before.length === 0, rows };
    },
  });
  const offered = !!(push.configured || push.fcm?.configured || push.any_configured);
  if (!offered) return null;
  const t = test.data;
  return (
    <div className={styles.control}>
      <div className={styles.actions}>
        <Button kind="tertiary" size="md" onClick={() => test.mutate()} disabled={test.isPending}>
          Send a test
        </Button>
      </div>
      <Outcome busy={test.isPending} busyText="Sending…" error={test.error} />
      {t && !test.isPending ? (
        <div role="status" aria-label="The test’s answer" className={styles.block}>
          {t.none ? (
            <p>No phone is registered, so the test went nowhere. {NO_PHONES}</p>
          ) : t.rows.length === 0 ? (
            <p className={styles.bad}>No phone received the test.</p>
          ) : (
            <ul className={styles.rows}>
              {t.rows.map((r) => (
                <li key={r.device} className={styles.row}>
                  <span className={r.ok ? undefined : styles.bad}>
                    <span className={styles.mono}>…{r.device}</span>: {r.said}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}
