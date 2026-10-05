// New for agent-router: Settings → Network, "Listen on your network" (assets/index.html, `netForm`;
// ST-C65 to ST-C72). The switch is the router's own state from `/api/auth`, read again after every
// answer, refusals included; turning it off asks first, and says so when the reader would lock
// themselves out.
import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { CodeSnippet } from '@carbon/react';
import { Link } from 'react-router';
import { keys } from '~/app/query';
import { Page } from '~/components/Page';
import { setNetwork, useAuth, useReread } from '~/data/settings';
import { settingsPath } from '~/lib/routes';
import { Switch } from './ClaudeCode';
import { Confirm, CopyButton, Explain, ReadFailed, Reading } from './parts';
import { networkWhere } from './rules';
import styles from './Settings.module.scss';

export default function NetworkPage() {
  const auth = useAuth();
  const reread = useReread();
  const [asking, setAsking] = useState(false);
  const a = auth.value;
  const on = !!a?.remote_bind;
  const write = useMutation({
    mutationFn: async (enable: boolean) => {
      try {
        await setNetwork(enable);
      } finally {
        // Even a refusal can follow a change that happened: "listening changed, but it could not be
        // saved for next time" (ST-C66).
        await reread(keys.auth);
      }
    },
  });

  if (!a) return auth.unreachable ? <ReadFailed what="Could not read how the router listens" error={auth.error} /> : <Reading what="Reading how the router listens…" />;

  const passwordFirst = !on && !a.configured;
  return (
    <Page.Section title="Listen on your network">
      <p className={styles.lead}>{on ? 'on' : 'this Mac only'}</p>
      <Switch
        label="Listen on your network"
        on={on}
        // Off is always offered; on, only once a password is set (ST-C68).
        disabled={passwordFirst}
        onToggle={(next) => {
          write.reset();
          if (next) write.mutate(true);
          else setAsking(true);
        }}
        busy={write.isPending}
        error={write.error}
        done={write.isSuccess ? (on ? 'On — reachable from your network.' : 'Off — this Mac only.') : null}
      >
        <Explain>
          Off, the router answers only on this Mac. On, it also listens on your local network, so the phone apps — and a Claude Code on another machine —
          can reach it; every one of them is asked for the password. It takes effect at once, with nothing restarted and no session dropped, and it is
          remembered across a reboot.
        </Explain>
        {passwordFirst ? (
          <Explain className={styles.warn}>
            Set a password first, on the <Link to={settingsPath('password')}>Password</Link> page — until one exists, other devices are refused anyway.
          </Explain>
        ) : null}
        {a.env_override ? (
          <Explain className={styles.warn}>
            <code>AGENT_ROUTER_BIND</code> is set in this router’s environment; it decides the next time the router starts, whatever this switch says.
          </Explain>
        ) : null}
      </Switch>

      <div className={styles.block}>
        {on && a.lan_url ? (
          <>
            <Explain>Reachable at:</Explain>
            <div className={styles.snippetRow}>
              <CodeSnippet type="single" hideCopyButton className={styles.snippet} aria-label="The router’s address on your network">
                {a.lan_url}
              </CodeSnippet>
              <CopyButton text={a.lan_url} what="the address" />
            </div>
          </>
        ) : (
          <Explain>{networkWhere(on, a.lan_url)}</Explain>
        )}
      </div>

      <Confirm
        open={asking}
        title="Stop listening on your network?"
        label="Listen on your network"
        confirmText="Turn off"
        busy={write.isPending}
        error={write.error}
        onClose={() => setAsking(false)}
        onConfirm={() => write.mutate(false, { onSuccess: () => setAsking(false) })}
      >
        <p>The router answers only on this Mac from then on: the phones and any Claude Code on another machine lose it until it is turned on again here.</p>
        {a.from_loopback === false ? (
          <p className={styles.bad}>
            You are reading this from another machine. This browser loses the router at once, and cannot turn it back on from here.
          </p>
        ) : null}
      </Confirm>
    </Page.Section>
  );
}
