// New for agent-router: Settings → Claude Code. Start at login, route every project, route one
// project, and the editor wrapper, each switch showing what the router reports and each write its
// own outcome beside it (assets/index.html, `renderSetup`; ST-C53 to ST-C64). Never read, and never
// offered, on a router that has not said it is local (ST-C53, ST-C101).
import { useId, useState, type FormEvent, type ReactNode } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Button, Form, Tag, TextInput, Toggle } from '@carbon/react';
import { Page } from '~/components/Page';
import { settingsKeys, setRouting, setService, setWrapper, setWrapperAuto, useAuth, useReread, useSetup, useShim, type Shim, type ShimTarget } from '~/data/settings';
import { ago } from '~/model/format';
import { readsSetup } from '~/model/settings';
import { Explain, Outcome, ReadFailed, Reading } from './parts';
import { projectRoot, setupHeading, WRAPPER_SAID, wrapperState } from './rules';
import styles from './Settings.module.scss';

/// A switch with its explanation and its outcome beside it (ST-C4, ST-C9). It shows the state the
/// router reports and nothing else: the reader's press sends the write, and the switch moves only
/// when the state read back after it moves (ST-C13).
export function Switch({
  label,
  on,
  onToggle,
  disabled = false,
  busy = false,
  error,
  done,
  children,
}: {
  label: string;
  on: boolean;
  onToggle: (next: boolean) => void;
  disabled?: boolean;
  busy?: boolean;
  error?: unknown;
  done?: string | null;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <div className={styles.control}>
      <Toggle
        id={`${id}-switch`}
        labelText={label}
        labelA="Off"
        labelB="On"
        toggled={on}
        disabled={disabled || busy}
        onToggle={(next: boolean) => onToggle(next)}
        aria-describedby={`${id}-why`}
      />
      <div id={`${id}-why`} className={styles.controlWhy}>
        {children}
      </div>
      <Outcome busy={busy} error={error} done={done} />
    </div>
  );
}

export default function ClaudeCodePage() {
  const auth = useAuth();
  const local = readsSetup(auth.value);
  const setup = useSetup(local);
  const shim = useShim(local);
  const reread = useReread();

  const s = setup.value;
  const registered = !!s?.service?.registered;
  const global = (s?.managed ?? []).find((m) => m.scope === 'global');
  const projects = (s?.managed ?? []).filter((m) => m.scope === 'project');

  const service = useMutation({
    mutationFn: async (enable: boolean) => {
      try {
        await setService(enable);
      } finally {
        await reread(settingsKeys.setup);
      }
    },
  });
  const routing = useMutation({
    mutationFn: async (enable: boolean) => {
      try {
        return (await setRouting(enable)).detail ?? null;
      } finally {
        await reread(settingsKeys.setup);
      }
    },
  });

  if (setup.unreachable) return <ReadFailed what="Could not read the Claude Code setup" error={setup.error} />;
  if (!s) return <Reading what="Reading the Claude Code setup…" />;

  return (
    <>
      <Page.Section title="Claude Code">
        <p className={styles.lead}>{setupHeading(registered, !!global, projects.length)}</p>

        {s.service?.supported ? (
          <Switch
            label="Start at login"
            on={registered}
            onToggle={(next) => service.mutate(next)}
            busy={service.isPending}
            error={service.error}
            done={service.isSuccess ? (registered ? 'On — the router starts at login.' : 'Off — the router no longer starts at login.') : null}
          >
            <Explain>Runs in the background and restarts if it crashes.</Explain>
          </Switch>
        ) : (
          <div className={styles.control}>
            <span className={styles.controlLabel}>Start at login</span>
            <Explain>
              Not supported on {s.service?.platform || 'this platform'} yet: there is no service manager here for the router to register with.
            </Explain>
          </div>
        )}

        <Switch
          label="Route every project"
          on={!!global}
          onToggle={(next) => routing.mutate(next)}
          busy={routing.isPending}
          error={routing.error}
          done={routing.isSuccess ? routing.data : null}
        >
          <Explain>
            Points every project at this router by writing <code>ANTHROPIC_BASE_URL</code> into{' '}
            <code className={styles.path}>{s.global_settings_path || 'Claude Code’s global settings file'}</code>.
          </Explain>
        </Switch>

        <Projects projects={projects} />

        <Explain>
          Only <code>env.ANTHROPIC_BASE_URL</code> is ever written. Everything else in your settings file is left as it is, and turning routing off puts back
          whatever was there before — unless you have changed it yourself since, in which case it is left alone.
        </Explain>
      </Page.Section>

      {shim.unreachable ? <ReadFailed what="Could not read the editor wrapper’s state" error={shim.error} /> : null}
      {shim.value && (shim.value.targets ?? []).length > 0 ? <Wrapper shim={shim.value} /> : null}
    </>
  );
}

/// The projects routed one by one, each with its own Stop routing, and the field to route one more
/// (ST-C58, ST-C59).
function Projects({ projects }: { projects: readonly { readonly path: string }[] }) {
  const reread = useReread();
  const [dir, setDir] = useState('');
  const id = useId();
  const add = useMutation({
    mutationFn: async (project: string) => {
      try {
        return (await setRouting(true, project)).detail ?? null;
      } finally {
        await reread(settingsKeys.setup);
      }
    },
    // Routed: the field has done its job. Refused: it keeps what was typed, to be fixed in place.
    onSuccess: () => setDir(''),
  });
  const stop = useMutation({
    mutationFn: async (settingsPath: string) => {
      try {
        return (await setRouting(false, projectRoot(settingsPath))).detail ?? null;
      } finally {
        await reread(settingsKeys.setup);
      }
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const project = dir.trim();
    if (project) add.mutate(project);
  };
  return (
    <div className={styles.block}>
      {projects.length > 0 ? (
        <ul className={styles.rows} aria-label="Projects routed one by one">
          {projects.map((p) => (
            <li key={p.path} className={styles.row}>
              <span className={styles.rowText}>
                <span className={styles.controlLabel}>Project</span>
                <code className={styles.path}>{p.path}</code>
              </span>
              <Button kind="danger--tertiary" size="md" onClick={() => stop.mutate(p.path)} disabled={stop.isPending}>
                Stop routing
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
      {/* The row a Stop routing came from is gone once it reads back, so its outcome is said here. */}
      <Outcome busy={stop.isPending} error={stop.error} done={stop.isSuccess ? stop.data : null} />
      <Form className={styles.inlineForm} aria-label="Route a single project" onSubmit={submit}>
        <TextInput
          id={`${id}-project`}
          labelText="Route a single project instead"
          placeholder="/path/to/project"
          autoComplete="off"
          spellCheck={false}
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          helperText="The project’s directory: its .claude/settings.local.json is the file written."
        />
        <Button type="submit" kind="tertiary" size="md" disabled={add.isPending || dir.trim() === ''}>
          Route this project
        </Button>
      </Form>
      <Outcome busy={add.isPending} error={add.error} done={add.isSuccess ? add.data : null} />
    </div>
  );
}

/// The editor wrapper, offered once an editor extension is found (ST-C60 to ST-C63).
function Wrapper({ shim: w }: { shim: Shim }) {
  const reread = useReread();
  const available = !!w.available;
  const last = (w.auto_log ?? [])[0];
  const auto = useMutation({
    mutationFn: async (enabled: boolean) => {
      try {
        await setWrapperAuto(enabled);
        return enabled ? WRAPPER_SAID.on : WRAPPER_SAID.off;
      } finally {
        await reread(settingsKeys.shim);
      }
    },
  });
  return (
    <Page.Section title="Type into editor sessions">
      <Explain>
        Lets the live view send messages to sessions VS Code starts. It takes effect for sessions started afterwards; ones already open keep their old
        stdio.
      </Explain>
      {!available ? <Explain className={styles.bad}>The wrapper binary is missing next to agent-router, so nothing can be installed.</Explain> : null}
      <Switch
        label="Manage it automatically"
        on={!!w.auto}
        // It can always be turned off; on, only while there is a wrapper to install (ST-C61).
        disabled={!available && !w.auto}
        onToggle={(next) => auto.mutate(next)}
        busy={auto.isPending}
        error={auto.error}
        done={auto.isSuccess ? auto.data : null}
      >
        <Explain>
          Every extension update ships a fresh, unwrapped binary; with this on, the router wraps it within seconds of VS Code installing it.
          {last ? (
            <>
              {' '}
              Last: {last.what} · {ago(last.at)}.
            </>
          ) : null}
        </Explain>
      </Switch>
      <ul className={styles.rows} aria-label="Extension binaries">
        {(w.targets ?? []).map((t) => (
          <Target key={t.path} target={t} available={available} />
        ))}
      </ul>
    </Page.Section>
  );
}

/// One extension binary: its extension, its state, and Install or Remove for the path exactly as the
/// listing gave it. Install only while the wrapper binary is there; Remove always (ST-C62).
function Target({ target, available }: { target: ShimTarget; available: boolean }) {
  const reread = useReread();
  const installed = target.state === 'installed';
  const act = useMutation({
    mutationFn: async (action: 'install' | 'remove') => {
      try {
        await setWrapper(target.path, action);
        return WRAPPER_SAID[action];
      } finally {
        await reread(settingsKeys.shim);
      }
    },
  });
  return (
    <li className={styles.row}>
      <span className={styles.rowText}>
        <code className={styles.path}>{target.extension}</code>
        <span className={styles.inline}>
          <Tag type={installed ? 'green' : target.state === 'inconsistent' ? 'warm-gray' : 'gray'} size="sm">
            {wrapperState(target.state)}
          </Tag>
        </span>
      </span>
      {installed ? (
        <Button kind="danger--tertiary" size="md" onClick={() => act.mutate('remove')} disabled={act.isPending}>
          Remove
        </Button>
      ) : available ? (
        <Button kind="tertiary" size="md" onClick={() => act.mutate('install')} disabled={act.isPending}>
          Install
        </Button>
      ) : null}
      <div className={styles.rowOutcome}>
        <Outcome busy={act.isPending} error={act.error} done={act.isSuccess ? act.data : null} />
      </div>
    </li>
  );
}
