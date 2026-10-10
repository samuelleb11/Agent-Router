// New for agent-router: the pieces every Settings page is built from. A write's progress and its
// outcome sit beside the control that made it (ST-C9); a confirmation is an in-page danger modal,
// never `confirm()` (ST-C18, SH-C27).
import type { ReactNode } from 'react';
import { InlineLoading, InlineNotification, Modal } from '@carbon/react';
import { Refusal } from '~/components/Refusal';
import { refusalText } from '~/lib/errors';
import type { Segment } from '~/model/settings';
import styles from './Settings.module.scss';

/// A write's progress and outcome, beside its control: working while it runs, then the router's
/// refusal in its words, or what the write did (ST-C9, ST-C10). A refusal is never a success: when
/// there is an error, `done` is not shown.
export function Outcome({ busy, busyText = 'Working…', error, done, refusedTitle }: { busy?: boolean; busyText?: string; error?: unknown; done?: string | null | false; refusedTitle?: string }) {
  if (busy) return <InlineLoading className={styles.outcome} description={busyText} status="active" />;
  if (error) {
    return (
      <div className={styles.outcome}>
        <Refusal error={error} title={refusedTitle ?? 'Refused'} />
      </div>
    );
  }
  if (!done) return null;
  return (
    <div className={styles.outcome} role="status">
      <InlineNotification kind="success" lowContrast hideCloseButton title={done} />
    </div>
  );
}

/// The words of a note, from the router's notes (`~/model/settings`): text, code and emphasis, never
/// markup (SH-C26).
export function Words({ segments }: { segments: readonly Segment[] }) {
  return (
    <>
      {segments.map((s, i) =>
        typeof s === 'string' ? <span key={i}>{s}</span> : 'code' in s ? <code key={i}>{s.code}</code> : <em key={i}>{s.em}</em>,
      )}
    </>
  );
}

/// A sentence that explains the field or switch beside it, as visible text (ST-C4).
export function Explain({ id, children, className }: { id?: string; children: ReactNode; className?: string }) {
  return (
    <p id={id} className={[styles.explain, className].filter(Boolean).join(' ')}>
      {children}
    </p>
  );
}

/// A danger modal that asks before something that cannot simply be switched back: it names what
/// goes and what follows, and sends nothing until it is confirmed (ST-C18). A refusal stays inside
/// it in the router's words, so it can be read where it was asked.
export function Confirm({
  open,
  title,
  label,
  confirmText,
  busy = false,
  error,
  onConfirm,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  label?: string;
  confirmText: string;
  busy?: boolean;
  error?: unknown;
  onConfirm: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  if (!open) return null;
  return (
    <Modal
      open
      danger
      size="sm"
      modalHeading={title}
      modalLabel={label}
      primaryButtonText={busy ? 'Working…' : confirmText}
      secondaryButtonText="Cancel"
      primaryButtonDisabled={busy}
      preventCloseOnClickOutside
      onRequestClose={() => {
        if (!busy) onClose();
      }}
      onRequestSubmit={() => {
        if (!busy) onConfirm();
      }}
      className={styles.confirm}
    >
      <div className={styles.confirmBody}>
        {children}
        {error ? <Refusal error={error} /> : null}
      </div>
    </Modal>
  );
}

/// A read that failed, said as the failure it is with the router's reason, never as an empty page
/// (SH-C22).
export function ReadFailed({ what, error }: { what: string; error: unknown }) {
  return <InlineNotification kind="error" lowContrast hideCloseButton title={what} subtitle={refusalText(error)} role="alert" className={styles.readFailed} />;
}

/// A read still on its way.
export function Reading({ what }: { what: string }) {
  return <InlineLoading description={what} status="active" />;
}

/// A label and its value, for a card's facts.
export function Facts({ children, label }: { children: ReactNode; label?: string }) {
  return (
    <dl className={styles.facts} aria-label={label}>
      {children}
    </dl>
  );
}

export function Fact({ term, children, mono = false }: { term: string; children: ReactNode; mono?: boolean }) {
  return (
    <>
      <dt>{term}</dt>
      <dd className={mono ? styles.mono : undefined}>{children}</dd>
    </>
  );
}
