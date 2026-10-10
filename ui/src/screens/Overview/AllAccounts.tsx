// New for agent-router: All accounts (assets/index.html, `renderAllAccounts`): every account at once,
// whatever the page is scoped to (OV-C87), and nothing at all when the forecast lists none (OV-C86).
import { Tile } from '@carbon/react';
import { StatGrid } from '~/components/StatGrid';
import { StatTile, type Tone } from '~/components/StatTile';
import { UsageMeter } from '~/components/UsageMeter';
import type { Reading } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { relative, when } from '~/model/format';
import { allAccounts, fcLine, type Forecast, type Tone as LineTone, type Window } from '~/model/forecast';
import { ForecastLine, Panel } from './parts';
import styles from './Overview.module.scss';

const toneOf = (t: LineTone): Tone | undefined => (t === 'bad' ? 'bad' : t === 'warn' ? 'warn' : undefined);

/// A card's window: its meter with percentage and reset, "—" for a window rolled over or never read,
/// and its forecast line (OV-C94).
function CardWindow({ label, w, now, tone }: { label: string; w: Window | null | undefined; now: number; tone: 'accent' | 'slate' }) {
  const live = !!w && !w.expired && typeof w.used === 'number';
  const reset = w && w.reset_at != null ? w.reset_at * 1000 : null;
  const line = fcLine(w, now);
  return (
    <UsageMeter label={label} used={live ? (w!.used as number) * 100 : null} resets={live && reset ? `resets ${relative(reset, now)}` : undefined} resetsAt={live && reset ? when(reset) : undefined} tone={tone}>
      {line && <ForecastLine line={line} />}
    </UsageMeter>
  );
}

export function AllAccounts({ forecast, now, busy }: { forecast: Reading<Forecast> | null; now: number; busy: boolean }) {
  const all = allAccounts(forecast?.value, now);
  if (!all) return null;
  const tiles = [all.spend, all.tokens, all.requests, all.headroom, all.pace];
  const Person = CONCEPTS.account.icon;
  const list = forecast?.value.accounts ?? [];
  return (
    <Panel concept="account" title="All accounts" caption={all.caption} busy={busy}>
      <StatGrid label="Every account">
        {tiles.map((t) => (
          <StatTile key={t.label} label={t.label} value={t.value === '—' ? null : t.value} detail={t.sub} tone={toneOf(t.tone)} />
        ))}
      </StatGrid>
      {all.first && (
        // The first to run out, by its label, with its window and its time, as text (OV-C93).
        <p className={styles.note} data-first-to-go="">
          First to run out: {all.first.name}'s {all.first.window} window, {all.first.when}
          {all.first.verdict === 'tight' ? ' — too close to call' : ''}.
        </p>
      )}
      <div className={styles.cards}>
        {all.cards.map((c) => {
          const a = list.find((x) => x.id === c.id);
          return (
            <Tile key={c.id} className={styles.card} data-account-card={c.id}>
              <p className={styles.cardName}>
                <Person aria-hidden="true" style={{ fill: paint(CONCEPTS.account.colour) }} />
                {c.name}
                {c.active && <span className={styles.activeTag}>active</span>}
              </p>
              <p className={styles.note}>{c.sub}</p>
              <CardWindow label="5-hour" w={a?.five_hour} now={now} tone="accent" />
              <CardWindow label="7-day" w={a?.seven_day} now={now} tone="slate" />
              <p className={styles.cardFoot}>
                <span>{c.cost}</span>
                <span>{c.read}</span>
                <span>{c.requests}</span>
              </p>
            </Tile>
          );
        })}
      </div>
    </Panel>
  );
}
