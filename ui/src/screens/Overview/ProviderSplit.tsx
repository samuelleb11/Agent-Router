// New for agent-router: a total's split by provider, under its tile (OV-C226). A thin bar of the
// providers' shares over a line per provider — its mark, its figure and its share — so the split reads
// at a glance and every figure is on the page as text. The bar only keys the line: it is hidden from a
// screen reader, which hears the lines, and it takes no focus, so six tiles add no tab stops.
import { ProviderMark } from '~/components/ProviderMark';
import { paint, providerColour } from '~/lib/concepts';
import type { ProviderPart } from './tiles';
import styles from './Overview.module.scss';

export function ProviderSplit({ label, parts }: { label: string; parts: readonly ProviderPart[] | null }) {
  if (!parts) return null;
  return (
    <div className={styles.byProvider} data-provider-split="">
      <span className={styles.byProviderBar} aria-hidden="true">
        {parts
          .filter((p) => p.value > 0)
          .map((p) => (
            <span key={p.provider} className={styles.byProviderPart} data-provider={p.provider} style={{ flexGrow: p.value, background: paint(providerColour(p.provider)) }} />
          ))}
      </span>
      <ul className={styles.byProviderList} aria-label={`${label}, by provider`}>
        {parts.map((p) => (
          <li key={p.provider} data-provider={p.provider}>
            <span className={styles.byProviderKey} style={{ background: paint(providerColour(p.provider)) }} aria-hidden="true" />
            {/* Spaces between the cells, so a line reads as words: "Kimi Code 200k 2%". */}
            <ProviderMark provider={p.provider} alone /> <span className={styles.byProviderFigure}>{p.figure}</span>
            {p.share && (
              <>
                {' '}
                <span className={styles.byProviderShare}>{p.share}</span>
              </>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
