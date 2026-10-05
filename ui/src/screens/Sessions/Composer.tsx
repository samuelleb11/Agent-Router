// New for agent-router: the live view's composer (assets/live.html, `#composer`, `deliver`, Stop and
// btw) on Carbon. It belongs to one open session (SE-C146): the screen around it is keyed on the
// session's id, its draft is kept under that id (SE-C192), and every request it makes names the
// session it was drawn for.
import { useId, useLayoutEffect, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { Button, TextArea, Toggletip, ToggletipButton, ToggletipContent } from '@carbon/react';
import { Information } from '@carbon/icons-react';
import { FormModal } from '~/components/FormModal';
import { ASIDE_TIMEOUT_MS, postLive } from '~/data/live';
import { asideNotice, notice, SIDE_QUESTION, stopNotice } from '~/model/session';
import { COMPOSING, failed, walk, type Browse } from './logic';
import { useKept } from './keep';
import { NoticeLine } from './Notice';
import { useNotice } from './useNotice';
import type { Deliver } from './QuestionForm';
import styles from './Sessions.module.scss';

/// The box grows with its text up to this, and shrinks back after a send (SE-C147).
export const GROW_PX = 120;

function fit(box: HTMLTextAreaElement | null) {
  if (!box) return;
  box.style.height = 'auto';
  box.style.height = `${Math.min(box.scrollHeight, GROW_PX)}px`;
}

export function Composer({
  session,
  sent,
  stoppable,
  deliver,
  echo,
  aside,
  follow,
}: {
  session: string;
  /// What the person has already said in this session, newest last: ArrowUp's history (SE-C155).
  sent: readonly string[];
  /// Whether the stream vouches for a status a Stop can act on (SE-C157).
  stoppable: boolean;
  deliver: Deliver;
  echo: (text: string, label?: string) => void;
  aside: (answer: string | null | undefined) => void;
  follow: () => void;
}) {
  const id = useId();
  const box = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useKept(`draft:${session}`, '');
  const [browse, setBrowse] = useState<Browse>(COMPOSING);
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [asking, setAsking] = useState(false);
  const [askOpen, setAskOpen] = useState(false);
  const [side, setSide] = useState('');
  const [said, say] = useNotice();
  const flying = useRef(false);

  // The box follows its text after every change: a draft kept from an earlier visit, a send, a
  // side question, a step through the history. It is measured here, once the new text is in the
  // DOM. A frame scheduled beside the state change could run before React committed it, measure
  // the old text, and leave the box tall after a send (SE-C147). A history step also puts the
  // caret at the end (SE-C156), for the same reason here and not in a frame.
  const caretToEnd = useRef(false);
  useLayoutEffect(() => {
    const b = box.current;
    fit(b);
    if (caretToEnd.current && b) {
      caretToEnd.current = false;
      b.setSelectionRange(b.value.length, b.value.length);
    }
  }, [draft]);

  /// Send trims the text, sends nothing when it is empty, and is disabled while it flies; a sent
  /// message is echoed at once and the box is cleared; a refused one keeps the text (SE-C148,
  /// SE-C150, SE-C153).
  const send = async () => {
    const text = draft.trim();
    if (!text || flying.current) return;
    flying.current = true;
    setSending(true);
    const r = await deliver(text);
    flying.current = false;
    setSending(false);
    if (r.ok) {
      setDraft('');
      setBrowse(COMPOSING);
    }
    say(r.notice);
  };

  const stop = async () => {
    setStopping(true);
    const r = await postLive('/api/live/interrupt', { session });
    setStopping(false);
    // "Stopped." for four seconds, else the router's words (SE-C158).
    say(r.ok ? stopNotice(200, r.body) : notice(failed('stop', r)));
  };

  /// A side question: asked of the session's context in a fork the session never sees, echoed at
  /// once, answered into the feed (SE-C159, SE-C160).
  const askAside = async (question: string, fromBox: boolean) => {
    const q = question.trim();
    if (!q) return;
    setAsking(true);
    echo(q, SIDE_QUESTION);
    follow();
    if (fromBox) setDraft('');
    say(notice('asking on the side — this replays the session once, usually under a minute', { ok: true }));
    const r = await postLive<{ answer?: string; took_ms?: number; cost?: number }>('/api/live/aside', { session, text: q }, ASIDE_TIMEOUT_MS);
    setAsking(false);
    if (r.ok) {
      aside(r.body.answer);
      follow();
      say(asideNotice(200, r.body));
    } else {
      say(notice(failed('aside', r)));
    }
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends, Shift+Enter starts a new line (SE-C147); a composition in progress keeps its Enter.
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
      return;
    }
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
    const t = e.currentTarget;
    const step = walk(e.key, sent, browse, { value: t.value, start: t.selectionStart, end: t.selectionEnd });
    if (!step) return;
    e.preventDefault();
    setBrowse(step.state);
    // The caret lands at the end (SE-C156), placed by the layout effect once the step's text is in.
    caretToEnd.current = true;
    setDraft(step.value);
  };

  const sideSubmit = (e: FormEvent) => {
    e.preventDefault();
    setAskOpen(false);
    void askAside(side, false);
  };

  return (
    <div className={styles.composer} data-composer="">
      <NoticeLine notice={said} onClose={() => say(null)} />
      <div className={styles.composeRow}>
        <TextArea
          ref={box}
          id={`${id}-message`}
          className={styles.message}
          labelText="Message to this session"
          hideLabel
          rows={1}
          placeholder="Message this session…"
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            fit(e.target);
          }}
          onKeyDown={onKey}
        />
        {/* Send stays while a turn runs: the session queues what is typed then (SE-C149). */}
        <Button size="md" className={styles.send} onClick={() => void send()} disabled={sending}>
          Send
        </Button>
      </div>
      <div className={styles.composeActions}>
        {stoppable && (
          <Button size="md" kind="secondary" className={styles.stop} onClick={() => void stop()} disabled={stopping}>
            Stop the turn
          </Button>
        )}
        <Button
          size="md"
          kind="tertiary"
          disabled={asking}
          onClick={() => {
            // The box's text is the question; with nothing typed it is asked for (SE-C159).
            if (draft.trim()) void askAside(draft, true);
            else {
              setSide('');
              setAskOpen(true);
            }
          }}
        >
          Side question
        </Button>
        <Toggletip align="top-end" className={styles.help}>
          <ToggletipButton label="What these do">
            <Information />
          </ToggletipButton>
          <ToggletipContent>
            <p>
              Enter sends and Shift+Enter starts a new line; a message sent while a turn runs waits in the session’s queue. Stop the turn interrupts the turn
              running now. A side question is answered from this session’s context, without interrupting it.
            </p>
          </ToggletipContent>
        </Toggletip>
      </div>
      <FormModal
        open={askOpen}
        title="Ask on the side"
        modalLabel="Side question"
        submitText="Ask"
        submitDisabled={side.trim() === ''}
        onSubmit={sideSubmit}
        onClose={() => setAskOpen(false)}
        focusId={`${id}-side`}
      >
        <p className={styles.muted}>Answered from this session’s context, without interrupting it.</p>
        <TextArea id={`${id}-side`} labelText="Your side question" rows={3} value={side} onChange={(e) => setSide(e.target.value)} />
      </FormModal>
    </div>
  );
}
