// New for agent-router: the live view's transcript (assets/live.html, `render`) as React elements,
// each row drawn by the model's reading of it (`feedRowView` in ~/model/session): by its kind before
// its role (SE-C76), in the order the stream delivered it, never re-sorted (SE-C73).
import { useId, useState, type ReactNode, type Ref } from 'react';
import { CodeSnippet } from '@carbon/react';
import { feedRowView, type FeedEntry, type FeedRowView } from '~/model/session';
import { Clamp } from './Clamp';
import { Markdown } from './Markdown';
import { QuestionForm, type Deliver } from './QuestionForm';
import styles from './Feed.module.scss';

/// A command line or its output in Carbon's multi-line snippet, its own fold off: the Clamp folds
/// it, at four lines (SE-C85).
function Code({ text, wrap, label }: { text: string; wrap: boolean; label: string }) {
  return (
    <CodeSnippet
      type="multi"
      wrapText={wrap}
      maxCollapsedNumberOfRows={0}
      maxExpandedNumberOfRows={0}
      minCollapsedNumberOfRows={0}
      aria-label={label}
      copyButtonDescription={`Copy the ${label.toLowerCase()}`}
    >
      {text}
    </CodeSnippet>
  );
}

/// A tool call as a terminal block — its name and duration over a raised command line, only Bash
/// with the "$" glyph (SE-C79) — and its result hung directly off it under OUT, an empty output
/// adding nothing (SE-C80).
function Terminal({ tool, result }: { tool: FeedRowView | null; result: FeedRowView | null }) {
  return (
    <>
      {tool && (
        <div className={styles.who}>
          <span className={styles.name}>{tool.who}</span>
          {tool.duration && <span className={styles.dur}>{tool.duration}</span>}
        </div>
      )}
      <div className={styles.term}>
        {tool && (
          <div className={styles.command} data-shell={tool.shell || undefined} data-part="command">
            <Clamp kind="term">
              <Code text={tool.text} wrap={false} label="Command" />
            </Clamp>
          </div>
        )}
        {result && result.text !== '' && (
          <div className={styles.output} data-part="output">
            <span className={styles.out}>OUT</span>
            <Clamp kind="term">
              <Code text={result.text} wrap label="Output" />
            </Clamp>
          </div>
        )}
      </div>
    </>
  );
}

/// Thinking, collapsed to "Thought for 2s ›", opened and closed on demand, never clamped (SE-C82).
function Thinking({ row }: { row: FeedRowView }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <>
      <button type="button" className={styles.thought} aria-expanded={open} aria-controls={id} onClick={() => setOpen(!open)}>
        {row.who}
      </button>
      {open && (
        <p id={id} className={styles.thinking}>
          {row.text}
        </p>
      )}
    </>
  );
}

/// The words over a rail row: "subagent", "✓ answered", "⌘ command", a role (SE-C83).
function Who({ row }: { row: FeedRowView }) {
  if (!row.who && !row.duration) return null;
  return (
    <div className={styles.who}>
      <span>{row.who}</span>
      {row.duration && <span className={styles.dur}>{row.duration}</span>}
    </div>
  );
}

/// Words as written, whitespace kept, never Markdown (SE-C78), folded at 11em.
const Plain = ({ text }: { text: string }) => (
  <Clamp kind="prose">
    <p className={styles.plain}>{text}</p>
  </Clamp>
);

interface Drawn {
  readonly key: string;
  /// The entries it draws, from `at`: two for a tool call and the result hung off it.
  readonly at: number;
  readonly took: number;
  readonly body: ReactNode;
  readonly className: string | undefined;
  readonly data?: Readonly<Record<string, string | undefined>>;
}

/// One entry's look, for the rows the stream delivered and the echoes of what the reader did.
function draw(
  entry: FeedEntry,
  next: FeedEntry | undefined,
  session: string,
  deliver: Deliver,
  question: string | null,
): { drawn: Omit<Drawn, 'key' | 'at' | 'took'>; took: number } {
  if (entry.kind === 'echo') {
    // What the reader sent, echoed until the transcript has it (SE-C150, SE-C160).
    return {
      took: 1,
      drawn: {
        className: styles.echo,
        body: (
          <>
            <div className={styles.who}>{entry.label}</div>
            <p className={styles.plain}>{entry.text}</p>
          </>
        ),
      },
    };
  }
  const row = feedRowView(entry.item);
  const sub = row.subagent ? { 'data-subagent': '' } : undefined;
  switch (row.kind) {
    case 'person':
      // The person's own words: the one filled bubble (SE-C75).
      return { took: 1, drawn: { className: styles.person, data: { 'data-kind': 'person' }, body: <Plain text={row.text} /> } };
    case 'assistant':
    case 'aside':
      // Prose, in Markdown (SE-C78): on the page, or on a subagent's rail.
      return {
        took: 1,
        drawn: {
          className: row.subagent ? styles.rail : styles.prose,
          data: { 'data-kind': row.kind, ...sub },
          body: (
            <>
              {(row.subagent || row.kind === 'aside') && <Who row={row} />}
              <Clamp kind="prose">
                <Markdown blocks={row.blocks ?? []} />
              </Clamp>
            </>
          ),
        },
      };
    case 'tool': {
      const result = next?.kind === 'row' ? feedRowView(next.item) : null;
      const joined = result?.kind === 'result' && result.subagent === row.subagent;
      return {
        took: joined ? 2 : 1,
        drawn: { className: styles.rail, data: { 'data-kind': 'tool', ...sub }, body: <Terminal tool={row} result={joined ? result : null} /> },
      };
    }
    case 'result':
      return { took: 1, drawn: { className: styles.rail, data: { 'data-kind': 'result', ...sub }, body: <Terminal tool={null} result={row} /> } };
    case 'thinking':
      return { took: 1, drawn: { className: styles.rail, data: { 'data-kind': 'thinking', ...sub }, body: <Thinking row={row} /> } };
    case 'interrupt':
      // Something that happened, not something anyone said (SE-C77).
      return { took: 1, drawn: { className: styles.interrupt, data: { 'data-kind': 'interrupt' }, body: row.text } };
    case 'ask':
      return {
        took: 1,
        drawn: {
          className: '',
          data: { 'data-kind': 'ask' },
          body: row.ask ? (
            <QuestionForm
              session={session}
              ask={row.ask}
              formKey={`${entry.item.t ?? ''}:${JSON.stringify(row.ask.questions ?? [])}`}
              deliver={deliver}
              held={question}
            />
          ) : null,
        },
      };
    case 'answer':
      return {
        took: 1,
        drawn: {
          className: styles.rail,
          data: { 'data-kind': 'answer', 'data-answer': '', ...sub },
          body: (
            <>
              <Who row={row} />
              <Plain text={row.text} />
            </>
          ),
        },
      };
    default:
      return {
        took: 1,
        drawn: {
          className: styles.rail,
          data: { 'data-kind': row.kind, ...sub },
          body: (
            <>
              <Who row={row} />
              <Plain text={row.text} />
            </>
          ),
        },
      };
  }
}

/// The feed: one session's rows and echoes. An entry's key is its identity, so a rebuilt feed —
/// a fresh connection's replay (SE-C69) — draws new rows, folds measured anew, rather than handing
/// the old rows' state to the new ones by position.
export function Feed({
  session,
  entries,
  deliver,
  keys,
  arrived,
  arrivedRef,
  question = null,
}: {
  session: string;
  entries: readonly FeedEntry[];
  deliver: Deliver;
  /// Each entry's key, kept by its identity.
  keys: (e: FeedEntry) => string;
  /// The entry an address arrived at, marked for as long as it is there.
  arrived: number | null;
  arrivedRef: Ref<HTMLDivElement>;
  /// Why a question form cannot be answered from here, in the router's words, else null (SE-C236,
  /// SE-C237).
  question?: string | null;
}) {
  const drawn: Drawn[] = [];
  for (let i = 0; i < entries.length; ) {
    const entry = entries[i];
    if (!entry) break;
    const { drawn: d, took } = draw(entry, entries[i + 1], session, deliver, question);
    drawn.push({ ...d, key: keys(entry), at: i, took });
    i += took;
  }
  return (
    <div className={styles.feed} data-feed="">
      {drawn.map((d) => {
        const here = arrived !== null && arrived >= d.at && arrived < d.at + d.took;
        return (
          <div
            key={d.key}
            ref={here ? arrivedRef : undefined}
            className={[styles.entry, d.className, here ? styles.arrived : ''].filter(Boolean).join(' ')}
            data-entry={d.at}
            aria-current={here ? 'true' : undefined}
            tabIndex={here ? -1 : undefined}
            {...d.data}
          >
            {d.body}
          </div>
        );
      })}
    </div>
  );
}
