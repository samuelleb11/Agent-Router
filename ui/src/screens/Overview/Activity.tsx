// New for agent-router: Activity over time (assets/index.html, `#activity`, `renderActivity`). The chart is
// Carbon Charts' StackedBarChart, whose library arrives in a chunk of its own (./ActivityChart) only when
// there are bars to draw; the legend, the caption and which empty it is are drawn here.
import type { Timeline } from '~/data/overview';
import { ActivityChart } from './ActivityChart';
import { activityCaption, activityEmpty, legend, perBucket, type Bucket } from './timeline';
import { Panel, Swatch } from './parts';
import styles from './Overview.module.scss';

export function Activity({ bk, timeline, now, busy }: { bk: readonly Bucket[]; timeline: Timeline; now: number; busy: boolean }) {
  const empty = activityEmpty(bk);
  return (
    <Panel concept="activity" title="Activity over time" caption={empty ? undefined : activityCaption(bk)} busy={busy}>
      {/* The legend keeps the stacking order, dims a family absent from the range, and filters
          nothing: it is not a control at all (OV-C109). */}
      <ul className={styles.legend} aria-label="Model families">
        {legend(bk).map((e) => (
          <li key={e.family} data-present={e.present ? 'true' : 'false'}>
            <Swatch family={e.family} />
            {e.family}
          </li>
        ))}
        <li className={styles.legendNote}>{perBucket(timeline.bucket_ms)}</li>
      </ul>
      {empty ? (
        <p className={styles.empty}>{empty}</p>
      ) : (
        <ActivityChart bk={bk} timeline={timeline} now={now} />
      )}
    </Panel>
  );
}
