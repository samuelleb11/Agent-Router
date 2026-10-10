// Copied from Northstar ui/src/components/FormModal.tsx at e9c1589.
// Changed: nothing. Its docs/13 is Northstar's console contracts, which bind here too (docs/web-console-carbon.md).
import { useId, type FormEvent, type ReactNode } from 'react';
import { ComposedModal, Form, ModalBody, ModalFooter, ModalHeader } from '@carbon/react';
import styles from './FormModal.module.scss';

export interface FormModalProps {
  open: boolean;
  /// The heading: "Add a forge", "Edit forge lab", "Link a repository".
  title: string;
  /// The small line above the heading — which page, or which row is being edited.
  modalLabel?: string;
  /// Submitted by the footer's primary button, by Enter in any field, and by nothing else.
  onSubmit: (e: FormEvent) => void;
  /// Cancel, the ×, and Escape. The caller sets `open` false.
  onClose: () => void;
  /// The primary button's word: "Add forge", "Save", "Link repository".
  submitText: string;
  /// Required fields are still empty, so there is nothing to send yet.
  submitDisabled?: boolean;
  /// A request is in flight.
  busy?: boolean;
  size?: 'xs' | 'sm' | 'md' | 'lg';
  /// The id of the field to focus when it opens. Without one Carbon focuses the close ×,
  /// which makes a keyboard user tab through the chrome to reach the first field.
  focusId?: string;
  /// The fields. They keep their own state in the caller, which is the thing that knows
  /// whether they are valid.
  children: ReactNode;
}

/// A form in a dialog: the shape every "add one of these" in settings takes.
///
/// **Why a dialog rather than a section at the foot of the page.** The settings pages each
/// grew an add-or-edit form below their table, so the page was a list of things and then,
/// after it, a form that was usually irrelevant — and on a page with several tables the
/// form was a long way from the table it filled. The button that opens this sits on the
/// heading of the section it adds to, where a reader looks for it.
///
/// **Why the caller keeps the state.** The footer's primary button lives outside the
/// `<form>` element, so it is bound to it by `form={id}` — the HTML attribute for exactly
/// this, which submits a form from a button anywhere in the document. That means this
/// component never has to know a field's value in order to submit it, and the caller,
/// which does know, says whether there is anything worth sending.
///
/// Mounted only while open, so every opening starts from the caller's initial state rather
/// than from whatever the last one was left in. On a phone it is the whole screen
/// (`docs/13-console-contracts.md` §7).
export function FormModal(props: FormModalProps) {
  return props.open ? <Dialog {...props} /> : null;
}

function Dialog({
  title,
  modalLabel,
  onSubmit,
  onClose,
  submitText,
  submitDisabled = false,
  busy = false,
  size = 'sm',
  focusId,
  children,
}: FormModalProps) {
  const id = useId();
  const formId = `form-${id.replace(/[^a-zA-Z0-9-]/g, '')}`;
  return (
    <ComposedModal
      open
      onClose={onClose}
      size={size}
      containerClassName={styles.container}
      // A half-typed credential or URL should not be lost to a stray click on the backdrop.
      preventCloseOnClickOutside
      aria-label={title}
      selectorPrimaryFocus={focusId ? `#${CSS.escape(focusId)}` : undefined}
    >
      <ModalHeader title={title} label={modalLabel} />
      <ModalBody hasForm>
        <Form id={formId} aria-label={title} onSubmit={onSubmit} className={styles.form}>
          {children}
        </Form>
      </ModalBody>
      <ModalFooter>
        <button type="button" className="cds--btn cds--btn--secondary" onClick={onClose} disabled={busy}>
          Cancel
        </button>
        {/* Bound to the form by id, so Enter in a field and this button do one thing. */}
        <button type="submit" form={formId} className="cds--btn cds--btn--primary" disabled={submitDisabled || busy}>
          {submitText}
        </button>
      </ModalFooter>
    </ComposedModal>
  );
}
