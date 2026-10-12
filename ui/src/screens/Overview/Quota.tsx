// New for agent-router: Quota headroom (assets/index.html, `#quotabox`, `renderQuota`, `renderRunway`),
// one block per provider with an account, side by side on a desk and stacked in a hand (OV-C224). Each
// block describes its provider's serving account whatever the page is scoped to (OV-C54): its mark and
// name, the account, its windows by their own names, their history and burn-downs, and its own
// headroom and first to run out, never set against another provider's windows (OV-C225). A provider
// with no account has no block.
import { Link } from 'react-router';
import { useId, useLayoutEffect, useRef, useState } from 'react';
import { BurnDown } from '~/components/BurnDown';
import { ProviderMark } from '~/components/ProviderMark';
import { Sparkline } from '~/components/Sparkline';
import { StatTile, type Tone } from '~/components/StatTile';
import { UsageMeter } from '~/components/UsageMeter';
import type { ProviderQuota, Snapshot } from '~/data/overview';
import { settingsPath } from '~/lib/routes';
import { DASH, ago, tod } from '~/model/format';
import { burnDownLayout, burnDowns, providerHeadroom, type ForecastAccount, type Tone as LineTone } from '~/model/forecast';
import { providerName } from '~/model/settings';
import type { Bucket } from './timeline';
import { ForecastLine, Panel, StatusTag } from './parts';
import { burnOf, forecastFor, history, quotaView, secondOf, servingName } from './quotaView';
import styles from './Overview.module.scss';

const percent = (v: number) => `${Math.round(v)}%`;
const toneOf = (t: LineTone): Tone | undefined => (t === 'bad' ? 'bad' : t === 'warn' ? 'warn' : undefined);

/// The burn-downs, side by side only when both fit at 250 px, each drawn at the width it is shown
/// (OV-C85), measured again when the block's width changes (OV-C172).
function BurnDowns({ account, now }: { account: ForecastAccount | null; now: number }) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(620);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => {
      const w = el.getBoundingClientRect().width;
      if (w > 0) setWidth(Math.round(w));
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  const wins = burnDowns(account);
  const { columns } = burnDownLayout(width, wins.length);
  return (
    <div ref={box} className={styles.burns} style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }} data-burn-downs={wins.length}>
      {wins.map((w) => (
        <BurnDown key={w.key} label={w.label} burn={burnOf(w.window)} now={now} tone={w.key === 'five_hour' ? 'accent' : 'slate'} formatTime={(t) => tod(t, now)} />
      ))}
    </div>
  );
}

/// One provider's block: its serving account's meters, status, history and burn-downs, and its own
/// headroom (OV-C224, OV-C225).
function Block({ block, snapshot, bk, now }: { block: ProviderQuota; snapshot: Snapshot; bk: readonly Bucket[]; now: number }) {
  const heading = useId();
  const name = providerName(block.provider);
  const fc = forecastFor(snapshot.forecast?.value.accounts, block.account);
  // `/api/quota` reads Anthropic's serving account: it stands in Anthropic's block alone.
  const live = block.provider === 'anthropic' ? snapshot.quota : null;
  const view = quotaView(live, block, fc, now);
  const hist = history(block.history, secondOf(block, fc), bk, now);
  const room = providerHeadroom(snapshot.forecast?.value, block.provider, now);
  const kept = live?.kept ? `kept from a read ${ago(live.at, now)}, the last one failed` : null;

  return (
    <section className={styles.block} aria-labelledby={heading} data-quota-provider={block.provider}>
      <div className={styles.panelHead}>
        <h3 id={heading} className={styles.h3}>
          <ProviderMark provider={block.provider} />
        </h3>
        <p className={styles.caption}>
          serving now:{' '}
          {/* The serving account's name leads to where it can be switched, edited or removed (SH-C17). */}
          <Link to={settingsPath('accounts')} data-serving="">
            {servingName(block.account)}
          </Link>
        </p>
      </div>
      {room && (
        // This provider's own headroom and first to run out, counted over its own windows (OV-C225).
        <div className={styles.room}>
          <StatTile label="Headroom left" value={room.tile.value === DASH ? null : room.tile.value} detail={room.tile.sub} tone={toneOf(room.tile.tone)} />
          {room.first && (
            // The first to run out, by its label, with its window and its time, as text (OV-C93).
            <p className={styles.note} data-first-to-go="">
              First to run out: {room.first.name}'s {room.first.window} window, {room.first.when}
              {room.first.verdict === 'tight' ? ' — too close to call' : ''}.
            </p>
          )}
        </div>
      )}
      {view ? (
        <>
          <div className={styles.meters}>
            {view.windows.map((w) => (
              <UsageMeter key={w.key} label={w.label} used={w.used} resets={w.resets} resetsAt={w.resetsAt} expired={w.expired} tone={w.tone}>
                {w.line && !w.expired && <ForecastLine line={w.line} />}
              </UsageMeter>
            ))}
          </div>
          <div className={styles.statusRow}>
            <span>Status</span>
            {view.status ? <StatusTag status={view.status} /> : <span>—</span>}
            <span className={styles.note} data-quota-source="">
              {view.source}
            </span>
            {kept && <span className={styles.note}>{kept}</span>}
          </div>
        </>
      ) : (
        // Which empty this is: no reading yet for this provider's serving account (OV-C145).
        <p className={styles.empty}>No quota reading yet for {name}'s serving account.</p>
      )}
      {hist && (
        <Sparkline
          label={`${name} quota history`}
          kind="lines"
          buckets={hist.buckets}
          series={hist.series}
          max={100}
          rules={[100, 90, 0]}
          format={percent}
          ends
          legend="percent used, over the window charted below"
        />
      )}
      <BurnDowns account={fc} now={now} />
    </section>
  );
}

export function Quota({ snapshot, bk, now, busy }: { snapshot: Snapshot; bk: readonly Bucket[]; now: number; busy: boolean }) {
  const blocks = snapshot.summary.providers ?? [];
  const forecastKept = snapshot.forecast?.kept ? `the forecast is kept from a read ${ago(snapshot.forecast.at, now)}` : null;

  return (
    <Panel concept="quota" title="Quota headroom" busy={busy} caption="each provider's account that is serving right now">
      {blocks.length ? (
        <div className={styles.blocks} data-quota-blocks={blocks.length}>
          {blocks.map((b) => (
            <Block key={b.provider} block={b} snapshot={snapshot} bk={bk} now={now} />
          ))}
        </div>
      ) : (
        // Which empty this is: no account at all, and where to add one (OV-C145).
        <p className={styles.empty}>
          No account yet — add one in <Link to={settingsPath('accounts')}>Settings → Accounts</Link>.
        </p>
      )}
      {forecastKept && <p className={styles.note}>{forecastKept}</p>}
    </Panel>
  );
}
