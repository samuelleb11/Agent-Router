// New for agent-router: Totals in scope (assets/index.html, `#headline`, `renderHeadline`): eight tiles,
// each with its meaning as visible text (OV-C98), and each total its split by provider under it
// (OV-C226). Spend is marked where it leaves requests out or rests on estimated tokens, the reason in
// its tip (OV-C229, OV-C232), and the cache's rates are each provider's own, in a tip (OV-C228).
import { Fragment } from 'react';
import { Button } from '@carbon/react';
import { DataTip } from '~/components/DataTip';
import { Sparkline } from '~/components/Sparkline';
import { StatGrid } from '~/components/StatGrid';
import { StatTile } from '~/components/StatTile';
import type { Snapshot } from '~/data/overview';
import { useCoarse } from '~/lib/viewport';
import { compact, tod } from '~/model/format';
import type { Scope } from '~/model/scope';
import type { Bucket } from './timeline';
import { cacheRates, headline, spendSplit, totalsCaption } from './tiles';
import { MarkedCost, Panel } from './parts';
import { listErrors } from './requestList';
import { ProviderSplit } from './ProviderSplit';
import styles from './Overview.module.scss';

const n = (v: number | null | undefined): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

export function Headline({
  snapshot,
  scope,
  bk,
  now,
  busy,
  onListErrors,
}: {
  snapshot: Snapshot;
  scope: Scope;
  bk: readonly Bucket[];
  now: number;
  busy: boolean;
  /// Sets the request filter to errors, returns the list to its first page and brings it into view
  /// (OV-C107).
  onListErrors: () => void;
}) {
  const coarse = useCoarse();
  const t = snapshot.summary.totals;
  const tiles = headline(t, snapshot.summary.latency, scope, now);
  const times = bk.map((b) => tod(b.t, now));
  const prompt = n(t.prompt_tokens);
  const split = spendSplit(t);
  const rates = cacheRates(t);

  return (
    <Panel title="Totals in scope" caption={totalsCaption(t, now)} busy={busy}>
      <StatGrid label="Totals in scope">
        {tiles.map((tile) => {
          const value = tile.value === '—' ? null : tile.value;
          // The total's split by provider, under it and its own picture (OV-C226).
          const providers = <ProviderSplit label={tile.label} parts={tile.byProvider} />;
          switch (tile.key) {
            case 'prompt':
              return (
                <StatTile key={tile.key} hero label={tile.label} value={value} detail={tile.detail} meaning={tile.meaning}>
                  {bk.length > 0 && <Sparkline label="Prompt tokens per bucket" buckets={times} series={[{ key: 'prompt', label: 'prompt tokens', values: bk.map((b) => b.prompt), colour: '--ar-tok' }]} format={compact} />}
                  {providers}
                </StatTile>
              );
            case 'spend':
              // Spend opens into its full split: a tap or the keyboard pins it, and the bar's own name
              // carries every figure (OV-C101).
              // Its value marked where it leaves requests out or rests on estimated tokens, each mark's
              // reason in its own tip (OV-C229, OV-C232).
              return (
                <StatTile key={tile.key} label={tile.label} value={value === null ? null : <MarkedCost text={value} marks={tile.marks} label={tile.label} />} detail={tile.detail} meaning={tile.meaning}>
                  <DataTip
                    className={styles.spendTarget}
                    role="group"
                    label={`Where the money went: ${split.map((s) => `${s.label} ${s.value}`).join('; ')}`}
                    tip={() => ({ title: 'Where the money went', rows: split })}
                  >
                    <dl className={styles.split}>
                      {split.slice(0, 2).map((s) => (
                        <Fragment key={s.label}>
                          <dt>{s.value}</dt>
                          <dd>{s.label}</dd>
                        </Fragment>
                      ))}
                    </dl>
                    <span className={styles.note}>the whole split — tap or focus</span>
                  </DataTip>
                  {providers}
                </StatTile>
              );
            case 'cache':
              return (
                <StatTile key={tile.key} label={tile.label} value={value} detail={tile.detail} meaning={tile.meaning}>
                  <span className={styles.cacheMeter} aria-hidden="true">
                    {prompt > 0 && <span className={styles.cacheFill} style={{ inlineSize: `${(100 * n(t.cache_read_tokens)) / prompt}%`, background: 'var(--ar-tok-3)' }} />}
                  </span>
                  {providers}
                </StatTile>
              );
            case 'writes':
              // Each provider's own cache rates, a tap or the keyboard away (OV-C228).
              return (
                <StatTile key={tile.key} label={tile.label} value={value} detail={tile.detail} meaning={tile.meaning}>
                  <DataTip
                    className={styles.spendTarget}
                    role="group"
                    label={`Cache rates: ${rates.map((r) => `${r.label} ${r.value}`).join('; ')}`}
                    tip={() => ({ title: 'Cache rates, of uncached input', rows: rates })}
                  >
                    <span className={styles.note}>each provider’s rates — tap or focus</span>
                  </DataTip>
                  {providers}
                </StatTile>
              );
            case 'output':
              return (
                <StatTile key={tile.key} label={tile.label} value={value} detail={tile.detail} meaning={tile.meaning}>
                  {bk.length > 0 && <Sparkline label="Output tokens per bucket" buckets={times} series={[{ key: 'output', label: 'output tokens', values: bk.map((b) => b.output), colour: '--ar-tok' }]} format={compact} />}
                  {providers}
                </StatTile>
              );
            case 'errors':
              return (
                <StatTile key={tile.key} label={tile.label} value={value} detail={tile.detail} meaning={tile.meaning} tone={tile.tone}>
                  {n(t.errors) > 0 && (
                    <Button kind="tertiary" size={coarse ? 'lg' : 'sm'} className={styles.tileButton} onClick={onListErrors}>
                      {listErrors(n(t.errors))}
                    </Button>
                  )}
                </StatTile>
              );
            default:
              return (
                <StatTile key={tile.key} label={tile.label} value={value} detail={tile.detail} meaning={tile.meaning} tone={tile.tone}>
                  {providers}
                </StatTile>
              );
          }
        })}
      </StatGrid>
    </Panel>
  );
}
