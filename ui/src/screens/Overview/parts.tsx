// New for agent-router: the small pieces the Overview's panels share: a panel with its heading and
// caption, a model family's swatch, a status tag and a forecast line.
import { WarningAlt } from '@carbon/icons-react';
import { useId, type ReactNode } from 'react';
import { CONCEPTS, FAMILY_COLOUR, paint, type Concept } from '~/lib/concepts';
import type { Family } from '~/model/format';
import type { Line } from '~/model/forecast';
import type { Status } from './quotaView';
import styles from './Overview.module.scss';

/// One section of the Overview: its heading with its concept's icon (OV-C4), the caption that says
/// what it measures or how to use it as visible text beside the heading, never in a tooltip
/// (OV-C2), and its content.
export function Panel({
  id,
  concept,
  title,
  caption,
  busy,
  children,
  className,
}: {
  id?: string;
  /// Its concept, whose icon it wears; none for a heading no concept names.
  concept?: Concept;
  title: string;
  caption?: ReactNode;
  /// Its figures belong to the previous scope while this one is read.
  busy?: boolean;
  children?: ReactNode;
  className?: string;
}) {
  const heading = useId();
  const Icon = concept ? CONCEPTS[concept].icon : null;
  return (
    <section id={id} className={[styles.panel, className].filter(Boolean).join(' ')} aria-labelledby={heading} aria-busy={busy || undefined} data-panel={concept}>
      <div className={styles.panelHead}>
        <h2 id={heading} className={styles.h2}>
          {Icon && concept && <Icon aria-hidden="true" style={{ fill: paint(CONCEPTS[concept].colour) }} />}
          {title}
        </h2>
        {caption !== undefined && caption !== null && caption !== '' && <p className={styles.caption}>{caption}</p>}
      </div>
      {children}
    </section>
  );
}

/// A model family's swatch: a square in its colour, "other" a hollow ring (OV-C10).
export function Swatch({ family }: { family: Family }) {
  return <span className={styles.swatch} data-family={family} style={family === 'other' ? undefined : { background: paint(FAMILY_COLOUR[family]) }} aria-hidden="true" />;
}

/// The status tag: its word and its colour from the same field, the error icon only when red (OV-C59).
export function StatusTag({ status }: { status: Status }) {
  return (
    <span className={styles.status} data-tone={status.tone} data-status-tag="">
      {status.icon && <WarningAlt aria-hidden="true" style={{ fill: paint(CONCEPTS.errors.colour) }} />}
      {status.word}
    </span>
  );
}

/// A window's forecast line, red only for a crossing that survives a quiet week, with what it was
/// fitted on as visible text under it (OV-C68, OV-C76).
export function ForecastLine({ line }: { line: Line }) {
  return (
    <div data-forecast-line="">
      <p className={styles.line} data-tone={line.tone}>
        {line.tone === 'bad' && <WarningAlt aria-hidden="true" style={{ fill: paint(CONCEPTS.errors.colour) }} />}
        <span>{line.text}</span>
      </p>
      {line.basis && <span className={styles.basis}>{line.basis}</span>}
    </div>
  );
}
