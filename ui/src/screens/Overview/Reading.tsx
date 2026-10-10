// New for agent-router: What the model read beside What fills the context window, and By model beside
// Main thread vs subagents (assets/index.html, `renderRead`, `renderContext`, `renderModels`,
// `renderThreads`).
import { BarList } from '~/components/BarList';
import { ProviderMark } from '~/components/ProviderMark';
import { ShareBar } from '~/components/ShareBar';
import type { Summary, TabPart } from '~/data/overview';
import { costNote, family } from '~/model/format';
import type { Scope } from '~/model/scope';
import { CONTEXT_NOTE, contextParts, emptyWords, modelRows, readPanel, threadParts } from './panels';
import { Panel, Swatch } from './parts';
import styles from './Overview.module.scss';

/// The Context tab: the top's totals and models, with the context and threads read for it.
export function Reading({ summary, part, scope, busy }: { summary: Summary; part: Extract<TabPart, { tab: 'context' }>; scope: Scope; busy: boolean }) {
  const read = readPanel(summary.totals, scope);
  const context = contextParts(part.context);
  const contextTotal = context.reduce((a, p) => a + (p.value ?? 0), 0);
  const models = modelRows(summary.models);
  // What a marked model cost means, once under the list (OV-C229, OV-C232).
  const marked = costNote(models.map((m) => m.cost));
  return (
    <>
      <div className={styles.two} data-two="">
        <Panel concept="cache" title="What the model read" caption="cache vs. paid in full" busy={busy}>
          <ShareBar label="What the model read" parts={read.parts} total={read.total ?? undefined} of="of everything read" />
          <p className={styles.note}>{read.note}</p>
        </Panel>
        <Panel concept="contextWindow" title="What fills the context window" caption="estimate · by character share" busy={busy}>
          <ShareBar label="What fills the context window" parts={context} total={contextTotal || undefined} unit="est. tokens" of="of the window" rowBars />
          <p className={styles.note}>{CONTEXT_NOTE}</p>
        </Panel>
      </div>
      <div className={styles.two} data-two="">
        <Panel concept="model" title="By model" caption="prompt tokens read" busy={busy}>
          {models.length ? (
            // Each model tagged with the provider that served it, by its mark and name (OV-C231).
            <BarList label="By model" rows={models.map((m) => ({ ...m, mark: <Swatch family={family(m.key)} />, chips: <ProviderMark provider={m.provider} className={styles.chip} /> }))} />
          ) : (
            <p className={styles.empty}>No requests in scope.</p>
          )}
          {marked && (
            <p className={styles.note} data-cost-note="">
              {marked}
            </p>
          )}
        </Panel>
        <Panel concept="subagents" title="Main thread vs subagents" caption="share of prompt tokens" busy={busy}>
          <ShareBar label="Main thread vs subagents" parts={threadParts(part.threads)} of="of prompt tokens" />
          {!part.threads.length && <p className={styles.empty}>{emptyWords('models', scope)}</p>}
        </Panel>
      </div>
    </>
  );
}
