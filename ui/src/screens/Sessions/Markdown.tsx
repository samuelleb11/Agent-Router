// New for agent-router: the model's Markdown block tree (~/model/markdown) as React elements. The
// live view built HTML and escaped it (assets/live.html, `md`); here nothing is ever HTML, so
// nothing a transcript contains can become markup or an attribute (SE-C90). Northstar's
// Markdown.tsx is skipped as the plan says, and its rule kept: a React tree, never HTML.
import { CodeSnippet, Link, ListItem, UnorderedList } from '@carbon/react';
import type { Block, Inline } from '~/model/markdown';
import styles from './Feed.module.scss';

/// Only an http or https address is a link, opening in a new tab with no opener (SE-C91). The
/// model already reads no other scheme as a link; this holds the line again where it is drawn.
const LINKABLE = /^https?:/i;

function Inlines({ items }: { items: readonly Inline[] }) {
  return (
    <>
      {items.map((x, i) => {
        switch (x.type) {
          case 'text':
            return <span key={i}>{x.text}</span>;
          case 'code':
            // Verbatim (SE-C95): its text is text, nothing inside it read.
            return (
              <code key={i} className={styles.inlineCode}>
                {x.text}
              </code>
            );
          case 'bold':
            return (
              <strong key={i}>
                <Inlines items={x.children} />
              </strong>
            );
          case 'italic':
            return (
              <em key={i}>
                <Inlines items={x.children} />
              </em>
            );
          case 'link':
            return LINKABLE.test(x.href) ? (
              <Link key={i} href={x.href} target="_blank" rel="noopener noreferrer" className={styles.link}>
                <Inlines items={x.children} />
              </Link>
            ) : (
              <span key={i}>
                <Inlines items={x.children} />
              </span>
            );
        }
      })}
    </>
  );
}

const HEADINGS = { 3: 'h3', 4: 'h4', 5: 'h5', 6: 'h6' } as const;

function BlockView({ block }: { block: Block }) {
  switch (block.type) {
    case 'paragraph':
      return (
        <p className={styles.p}>
          <Inlines items={block.children} />
        </p>
      );
    case 'heading': {
      // Under the screen's own headings: `#` is the third level (SE-C98).
      const H = HEADINGS[block.level];
      return (
        <H className={styles.h}>
          <Inlines items={block.children} />
        </H>
      );
    }
    case 'list':
      // One flat list, bullets and numbers alike (SE-C99).
      return (
        <UnorderedList className={styles.list}>
          {block.items.map((item, i) => (
            <ListItem key={i}>
              <Inlines items={item} />
            </ListItem>
          ))}
        </UnorderedList>
      );
    case 'quote':
      return (
        <blockquote className={styles.quote}>
          <Inlines items={block.children} />
        </blockquote>
      );
    case 'table':
      // Scrolls sideways inside the column rather than widening it, its cells unwrapped (SE-C102).
      return (
        <div className={styles.tableScroll} role="region" aria-label="Table" tabIndex={0}>
          <table className={styles.table}>
            <thead>
              <tr>
                {block.head.map((c, i) => (
                  <th key={i}>
                    <Inlines items={c} />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((r, i) => (
                <tr key={i}>
                  {r.map((c, j) => (
                    <td key={j}>
                      <Inlines items={c} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case 'code':
      // Verbatim, and scrolled sideways rather than wrapped (SE-C93). Folding is the Clamp's
      // around the whole message, so Carbon's own is off.
      return (
        <CodeSnippet
          type="multi"
          className={styles.fence}
          maxCollapsedNumberOfRows={0}
          maxExpandedNumberOfRows={0}
          minCollapsedNumberOfRows={0}
          aria-label="Code"
          copyButtonDescription="Copy the code"
        >
          {block.text}
        </CodeSnippet>
      );
  }
}

/// Transcript Markdown, as elements whose text is text.
export function Markdown({ blocks, className }: { blocks: readonly Block[]; className?: string }) {
  return (
    <div className={[styles.md, className].filter(Boolean).join(' ')}>
      {blocks.map((b, i) => (
        <BlockView key={i} block={b} />
      ))}
    </div>
  );
}
