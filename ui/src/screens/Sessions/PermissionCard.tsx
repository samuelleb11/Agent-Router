// New for agent-router: the live view's permission card (assets/live.html, `drawPerm`,
// `answerPerm`) as a Carbon Callout. Never an ActionableNotification, which is an alertdialog that
// takes focus: an Enter meant for the composer could approve a prompt (SE-C130).
import { useId, useLayoutEffect, useRef, useState } from 'react';
import { Button, Callout, CodeSnippet } from '@carbon/react';
import { postLive } from '~/data/live';
import { notice, permissionAnswer, type Choice, type Notice, type PermissionCard as Card } from '~/model/session';
import { failed } from './logic';
import styles from './Sessions.module.scss';

/// What became of an answer, for the place the card leaves behind: done, and the card goes with a
/// success notice; gone, a 409, and the card is replaced by whatever waits now (SE-C126).
export type Answered = { readonly kind: 'done' | 'gone'; readonly requestId: string; readonly notice: Notice | null };

/// What the prompt would act on, in a wrapping snippet. Carbon's Callout requires its first
/// focusable child to be described by its title, and the snippet's focusable box takes no props of
/// its own, so it is described here, before the Callout checks.
function Subject({ text, titleId }: { text: string; titleId: string }) {
  const box = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    box.current?.querySelector('.cds--snippet-container')?.setAttribute('aria-describedby', titleId);
  }, [titleId]);
  return (
    <div ref={box} className={styles.subject}>
      <CodeSnippet type="multi" wrapText hideCopyButton maxCollapsedNumberOfRows={0} maxExpandedNumberOfRows={0} minCollapsedNumberOfRows={0} aria-label="What it would run">
        {text}
      </CodeSnippet>
    </div>
  );
}

/// The card for the prompt a session waits on: what waits and on what, and every choice as a
/// visible button — Allow, each of the prompt's own suggestions, Deny (SE-C121, SE-C123). It
/// answers the session it was drawn for (SE-C129), names its prompt (SE-C124), and only a press on
/// one of its buttons answers it (SE-C130). While an answer is in flight every button is disabled;
/// a refusal gives them back, in the router's words beside them (SE-C125).
export function PermissionCard({ session, card, onAnswered }: { session: string; card: Card; onAnswered: (a: Answered) => void }) {
  const titleId = useId();
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  const flying = useRef(false);
  const answer = async (choice: Choice) => {
    if (flying.current) return;
    flying.current = true;
    setBusy(true);
    setRefused(null);
    const r = await postLive('/api/live/answer', permissionAnswer(session, card, choice));
    flying.current = false;
    setBusy(false);
    if (r.ok) onAnswered({ kind: 'done', requestId: card.requestId, notice: notice(choice.done, { ok: true, for: 5000 }) });
    else if (r.status === 409) onAnswered({ kind: 'gone', requestId: card.requestId, notice: notice(failed('answer', r)) });
    else setRefused(failed('answer', r));
  };
  return (
    <Callout className={styles.card} kind="warning" lowContrast title={card.what} titleId={titleId} statusIconDescription="Waiting for permission">
      {card.more && <p className={styles.cardMore}>{card.more}</p>}
      {card.description && <p className={styles.cardWhy}>{card.description}</p>}
      {card.subject && <Subject text={card.subject} titleId={titleId} />}
      <div className={styles.choices}>
        {card.choices.map((c, i) => (
          <Button
            key={`${i}:${c.label}`}
            size="md"
            kind={i === 0 ? 'primary' : c.body.allow ? 'secondary' : 'danger--tertiary'}
            disabled={busy}
            aria-describedby={titleId}
            onClick={() => void answer(c)}
          >
            {c.label}
          </Button>
        ))}
      </div>
      {refused && (
        <p className={styles.cardRefused} role="status">
          {refused}
        </p>
      )}
    </Callout>
  );
}
