// Copied from Northstar ui/src/components/ReasonModal.tsx at e9c1589.
// Changed: the floor is the caller's, from the router's own rule, since the router sets no reason minimum the way Northstar's server does; a refusal always renders under the field, because the router's refusals name no field.
import { useId, useState, type ReactNode } from 'react';
import { ComposedModal, ModalBody, ModalFooter, ModalHeader, TextArea } from '@carbon/react';
import { Refusal } from '~/components/Refusal';
import styles from './ReasonModal.module.scss';

export interface ReasonModalProps {
  open: boolean;
  /// The heading: "Archive this session", "Remove the account".
  title: string;
  /// What the reason is for — the field's own label: "Reason to remove it".
  label: string;
  /// The fewest characters the router accepts, asked for BEFORE the request is sent, with the
  /// router's own number, so its refusal never has to happen. A console with a lower floor is
  /// lying about the rule. Without one, anything but an empty field is sent.
  min?: number;
  /// A request is in flight: the field and the buttons hold.
  busy?: boolean;
  /// The refusal, rendered under the field in the router's words.
  error?: unknown;
  /// Called with the TRIMMED reason, never under the floor. The caller closes the dialog on
  /// success and hands back `error` on refusal; the dialog itself never closes on send.
  onSend: (reason: string) => void;
  /// Cancel, the × and Escape. The caller sets `open` false.
  onClose: () => void;
  /// The small line above the heading: which session, which account.
  modalLabel?: string;
  /// The primary button's word. "Send" unless told otherwise.
  sendText?: string;
  /// A destructive send gets the danger button.
  danger?: boolean;
  /// What a good reason says, after the floor.
  hint?: string;
  /// Prose above the field: what this reason will be recorded beside.
  children?: ReactNode;
}

/// One reason dialog for every write that needs one.
///
/// The floor is counted as the person types and Send stays disabled under it, so the router's
/// refusal of a short reason is one this dialog never has to show. A refusal that comes anyway
/// is shown in the router's words. Mounted only while open, so each opening starts with an
/// empty field. On a phone it is the whole screen.
export function ReasonModal(props: ReasonModalProps) {
  return props.open ? <Dialog {...props} /> : null;
}

function Dialog({
  title,
  label,
  min = 1,
  busy = false,
  error = null,
  onSend,
  onClose,
  modalLabel,
  sendText = 'Send',
  danger = false,
  hint,
  children,
}: ReasonModalProps) {
  const [reason, setReason] = useState('');
  const id = useId();
  const trimmed = reason.trim();
  const short = trimmed.length < min;
  const counted = min > 1;
  return (
    <ComposedModal
      open
      onClose={() => {
        onClose();
      }}
      size="sm"
      containerClassName={styles.container}
      preventCloseOnClickOutside
      aria-label={title}
      selectorPrimaryFocus={`#${CSS.escape(id)}`}
    >
      <ModalHeader title={title} label={modalLabel} />
      <ModalBody hasForm>
        <div className={styles.body}>
          {children !== undefined && <div className={styles.prose}>{children}</div>}
          <TextArea
            id={id}
            labelText={label}
            rows={4}
            value={reason}
            disabled={busy}
            onChange={(e) => setReason(e.target.value)}
            helperText={
              counted || hint ? (
                <span className={`${styles.counter} ${counted && short ? styles.short : ''}`} data-reason-count={trimmed.length}>
                  {counted ? `${trimmed.length} / ${min} characters` : ''}
                  {counted && short ? ' — the router refuses a shorter reason' : ''}
                  {hint ? `${counted ? ' — ' : ''}${hint}` : ''}
                </span>
              ) : undefined
            }
          />
          {error ? <Refusal error={error} /> : null}
        </div>
      </ModalBody>
      <ModalFooter
        primaryButtonText={sendText}
        secondaryButtonText="Cancel"
        primaryButtonDisabled={short || busy}
        danger={danger}
        children={null}
        onRequestSubmit={() => {
          if (short || busy) return;
          onSend(trimmed);
        }}
      />
    </ComposedModal>
  );
}
