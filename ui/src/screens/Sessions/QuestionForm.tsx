// New for agent-router: the live view's `askForm` (assets/live.html) on Carbon. An AskUserQuestion
// the transcript recorded, drawn as the editor draws it: one tab per question, and one Submit that
// answers them together.
import { useId, useRef, useState } from 'react';
import { Button, Checkbox, CheckboxGroup, RadioButton, RadioButtonGroup, Tab, TabList, TabPanel, TabPanels, Tabs, TextInput, Tile } from '@carbon/react';
import { postLive, readPrompt } from '~/data/live';
import {
  PICK_FIRST,
  answerBody,
  answerOf,
  answerTarget,
  composeAnswers,
  questionTabs,
  type Ask,
  type AskQuestion,
  type Notice,
  type Prompt,
  type Selection,
} from '~/model/session';
import { failed } from './logic';
import { useKept } from './keep';
import { NoticeLine } from './Notice';
import { useNotice } from './useNotice';
import styles from './Feed.module.scss';

/// What is picked in one question: options by their place in the tool's list, and Other.
interface Picked {
  readonly picked: readonly number[];
  readonly otherOn: boolean;
  readonly other: string;
}

const NOTHING: Picked = { picked: [], otherOn: false, other: '' };
const OTHER = 'other';

/// The model's selection: the labels themselves, exactly as the tool wrote them, quotes and all
/// (SE-C110), and the Other text only while Other is chosen.
function selectionOf(q: AskQuestion, p: Picked): Selection {
  const labels = (q.options ?? []).map((o) => o.label);
  return { chosen: p.picked.map((i) => labels[i]).filter((l): l is string => l !== undefined), other: p.otherOn ? p.other : null };
}

/// A message sent for the session, as the composer's send makes it: echoed, and worded.
export type Deliver = (text: string, opts?: { readonly ask?: boolean }) => Promise<{ readonly ok: boolean; readonly notice: Notice | null }>;

/// One question form. It answers the session it was drawn in, whatever is open by the time the
/// router replies (SE-C119): `session` is fixed when the form is drawn, and what it says lands only
/// in its own place. What is half-picked is kept under its session (`formKey` names the transcript
/// row), so it stays with that session and follows the reader nowhere (SE-C192).
export function QuestionForm({ session, ask, formKey, deliver }: { session: string; ask: Ask; formKey: string; deliver: Deliver }) {
  const questions = ask.questions ?? [];
  const names = questionTabs(ask);
  // Each form's choices are its own: its groups are named from useId, never `q0` (SE-C108).
  const group = useId();
  const [picks, setPicks] = useKept<readonly Picked[]>(`form:${session}:${formKey}`, questions.map(() => NOTHING));
  const [tab, setTab] = useState(0);
  const [hint, setHint] = useState('or type a reply below');
  const [busy, setBusy] = useState(false);
  const [spent, setSpent] = useState(false);
  const [notice, say] = useNotice();
  const flying = useRef(false);

  const pickOf = (i: number): Picked => picks[i] ?? NOTHING;
  const set = (i: number, p: Picked) => setPicks(questions.map((_, j) => (j === i ? p : pickOf(j))));
  const selections = questions.map((q, i) => selectionOf(q, pickOf(i)));

  const submit = async () => {
    // One tap, one answer: a second tap while one is in flight does nothing (SE-C118).
    if (flying.current || spent) return;
    const { answers, lines } = composeAnswers(ask, selections);
    if (!lines.length) {
      setHint(PICK_FIRST);
      return;
    }
    flying.current = true;
    setBusy(true);
    setHint('answering…');
    const done = (words: string) => {
      flying.current = false;
      setBusy(false);
      setHint(words);
    };
    // What waits now decides where the answers may go (SE-C112, SE-C113, SE-C114).
    let live: Prompt | null = null;
    try {
      live = await readPrompt(session);
    } catch {
      live = null;
    }
    const target = answerTarget(live, ask);
    if (target.kind === 'refused') {
      done(target.text);
      return;
    }
    const answered = await postLive('/api/live/answer', answerBody(session, answers, target));
    if (answered.ok) {
      // Spent only now: the router has answered the prompt itself (SE-C115).
      setSpent(true);
      done('answered');
      return;
    }
    if (answered.status === null) {
      // Nothing was reached, so nothing is spent (SE-C117).
      done(`could not answer — ${answered.words}`);
      return;
    }
    // The router cannot answer directly: why, and the answers go as an ordinary message (SE-C116),
    // which leaves the composer's own draft alone (SE-C154).
    done(failed('form', answered));
    const sent = await deliver(lines.join('\n'), { ask: true });
    say(sent.notice);
  };

  return (
    <Tile className={styles.ask} data-spent={spent || undefined}>
      <Tabs selectedIndex={tab} onChange={({ selectedIndex }) => setTab(selectedIndex)}>
        <TabList aria-label="Questions">
          {questions.map((q, i) => (
            <Tab key={i}>
              {names[i]}
              {answerOf(q, selections[i]) ? <span aria-label=" (answered)"> ✓</span> : null}
            </Tab>
          ))}
        </TabList>
        <TabPanels>
          {questions.map((q, i) => (
            <TabPanel key={i}>
              <Question
                q={q}
                name={`${group}-q${i}`}
                picked={pickOf(i)}
                disabled={spent}
                onPick={(p) => set(i, p)}
              />
            </TabPanel>
          ))}
        </TabPanels>
      </Tabs>
      <div className={styles.askFoot}>
        <Button size="md" onClick={() => void submit()} disabled={busy || spent}>
          Submit answers
        </Button>
        <span className={styles.askHint} role="status">
          {hint}
        </span>
      </div>
      <NoticeLine notice={notice} onClose={() => say(null)} />
    </Tile>
  );
}

/// One question: one choice, or several when it says multiSelect (SE-C106); each option its label
/// and its description; and Other, always, with a field whose typing chooses it (SE-C107).
function Question({ q, name, picked, disabled, onPick }: { q: AskQuestion; name: string; picked: Picked; disabled: boolean; onPick: (p: Picked) => void }) {
  const options = q.options ?? [];
  const multi = !!q.multiSelect;
  const label = (o: { label: string; description?: string | null }) => (
    <span className={styles.option}>
      <span className={styles.optionLabel}>{o.label}</span>
      {o.description ? <span className={styles.optionWhy}>{o.description}</span> : null}
    </span>
  );
  const typed = (other: string) =>
    // Typing in Other chooses it, so what was typed is never dropped in silence (SE-C107).
    onPick(other.trim() ? { picked: multi ? picked.picked : [], otherOn: true, other } : { ...picked, other });
  const field = (
    <TextInput
      id={`${name}-other`}
      className={styles.other}
      labelText="Other answer"
      placeholder="Type your own answer…"
      value={picked.other}
      disabled={disabled}
      onChange={(e) => typed(e.target.value)}
    />
  );
  return (
    <div>
      <p className={styles.question}>{q.question}</p>
      {multi ? (
        <CheckboxGroup legendText="Choose any" disabled={disabled}>
          {options.map((o, j) => (
            <Checkbox
              key={j}
              id={`${name}-o${j}`}
              labelText={label(o)}
              checked={picked.picked.includes(j)}
              disabled={disabled}
              onChange={(_, { checked }) =>
                onPick({ ...picked, picked: checked ? [...picked.picked.filter((x) => x !== j), j] : picked.picked.filter((x) => x !== j) })
              }
            />
          ))}
          <Checkbox
            id={`${name}-other-on`}
            labelText="Other"
            checked={picked.otherOn}
            disabled={disabled}
            onChange={(_, { checked }) => onPick({ ...picked, otherOn: checked })}
          />
        </CheckboxGroup>
      ) : (
        <RadioButtonGroup
          name={name}
          legendText="Choose one"
          orientation="vertical"
          disabled={disabled}
          valueSelected={picked.otherOn ? OTHER : picked.picked[0] !== undefined ? `o${picked.picked[0]}` : ''}
          onChange={(v) => onPick(v === OTHER ? { ...picked, picked: [], otherOn: true } : { ...picked, picked: [Number(String(v).slice(1))], otherOn: false })}
        >
          {[
            ...options.map((o, j) => <RadioButton key={j} id={`${name}-o${j}`} value={`o${j}`} labelText={label(o)} />),
            <RadioButton key="other" id={`${name}-other-on`} value={OTHER} labelText="Other" />,
          ]}
        </RadioButtonGroup>
      )}
      {field}
    </div>
  );
}
