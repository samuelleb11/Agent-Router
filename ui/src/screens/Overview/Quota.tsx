// New for agent-router: Quota headroom (assets/index.html, `#quotabox`, `renderQuota`, `renderRunway`).
// It always describes the account serving now, whatever the page is scoped to (OV-C54).
import { Link } from 'react-router';
import { useLayoutEffect, useRef, useState } from 'react';
import { BurnDown } from '~/components/BurnDown';
import { Sparkline } from '~/components/Sparkline';
import { UsageMeter } from '~/components/UsageMeter';
import type { Snapshot } from '~/data/overview';
import { settingsPath } from '~/lib/routes';
import { ago, tod } from '~/model/format';
import { burnDownLayout, burnDowns } from '~/model/forecast';
import type { Bucket } from './timeline';
import { ForecastLine, Panel, StatusTag } from './parts';
import { burnOf, forecastFor, history, quotaView, servingName } from './quotaView';
import styles from './Overview.module.scss';

const percent = (v: number) => `${Math.round(v)}%`;

/// The burn-downs, side by side only when both fit at 250 px, each drawn at the width it is shown
/// (OV-C85), measured again when the panel's width changes (OV-C172).
function BurnDowns({ snapshot, now }: { snapshot: Snapshot; now: number }) {
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
  const account = forecastFor(snapshot.forecast?.value.accounts, snapshot.summary.account);
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

export function Quota({ snapshot, bk, now, busy }: { snapshot: Snapshot; bk: readonly Bucket[]; now: number; busy: boolean }) {
  const account = snapshot.summary.account;
  const fc = forecastFor(snapshot.forecast?.value.accounts, account);
  const view = quotaView(snapshot.quota, snapshot.summary.quota, fc, now);
  const hist = history(snapshot.summary.quota_history, bk, now);
  const kept = snapshot.quota?.kept ? `kept from a read ${ago(snapshot.quota.at, now)}, the last one failed` : null;
  const forecastKept = snapshot.forecast?.kept ? `the forecast is kept from a read ${ago(snapshot.forecast.at, now)}` : null;

  return (
    <Panel
      concept="quota"
      title="Quota headroom"
      busy={busy}
      caption={
        <>
          the account that is serving right now:{' '}
          {/* The serving account's name leads to where it can be switched, edited or removed (SH-C17). */}
          <Link to={settingsPath('accounts')} data-serving="">
            {servingName(account)}
          </Link>
        </>
      }
    >
      {view ? (
        <div className={styles.body}>
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
            {forecastKept && <span className={styles.note}>{forecastKept}</span>}
          </div>
        </div>
      ) : (
        // Which empty this is: no reading yet for the serving account (OV-C145).
        <p className={styles.empty}>No quota reading yet for the serving account.</p>
      )}
      {hist && (
        <Sparkline
          label="Quota history"
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
      <BurnDowns snapshot={snapshot} now={now} />
    </Panel>
  );
}
