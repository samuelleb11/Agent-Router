// New for agent-router: All accounts (assets/index.html, `renderAllAccounts`): every account at once,
// whatever the page is scoped to (OV-C87), and nothing at all when the forecast lists none (OV-C86). A
// Kimi Code account's card draws its second window under its real name (OV-C218), and every card names
// its provider by its mark and name beside the account's (OV-C231). Headroom is not totalled here: it
// is each provider's own, in its quota block (OV-C225). A cost that leaves requests out or rests on
// estimated tokens is marked, the reasons in its tip (OV-C229, OV-C232).
import { Tile } from '@carbon/react';
import { ProviderMark } from '~/components/ProviderMark';
import { StatGrid } from '~/components/StatGrid';
import { StatTile, type Tone } from '~/components/StatTile';
import { UsageMeter } from '~/components/UsageMeter';
import type { Reading } from '~/data/overview';
import { CONCEPTS, paint } from '~/lib/concepts';
import { relative, when } from '~/model/format';
import { allAccounts, fcLine, type Forecast, type Tone as LineTone, type Window } from '~/model/forecast';
import { providerWords } from './panels';
import { ForecastLine, MarkedCost, Panel } from './parts';
import styles from './Overview.module.scss';

const toneOf = (t: LineTone): Tone | undefined => (t === 'bad' ? 'bad' : t === 'warn' ? 'warn' : undefined);

/// A card's window: its meter with percentage and reset, "—" for a window rolled over or never read,
/// and its forecast line (OV-C94), which for a spent window says so until its reset (OV-C219).
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
  const tiles = [all.spend, all.tokens, all.requests, all.pace];
  const Person = CONCEPTS.account.icon;
  return (
    <Panel concept="account" title="All accounts" caption={all.caption} busy={busy}>
      <StatGrid label="Every account">
        {tiles.map((t) => (
          <StatTile key={t.label} label={t.label} value={t.value === '—' ? null : <MarkedCost text={t.value} marks={t.marks} label={t.label} />} detail={t.sub} tone={toneOf(t.tone)} />
        ))}
      </StatGrid>
      <div className={styles.cards}>
        {all.cards.map((c) => (
          <Tile key={c.id} className={styles.card} data-account-card={c.id}>
            <p className={styles.cardName}>
              <Person aria-hidden="true" style={{ fill: paint(CONCEPTS.account.colour) }} />
              {c.name}
              {/* Its provider, and while it signs that provider's new requests, "active for" it (OV-C231). */}
              <ProviderMark provider={c.provider} words={providerWords(c.provider, c.active)} className={c.active ? styles.activeTag : styles.chip} />
            </p>
            <p className={styles.note}>{c.sub}</p>
            {c.windows.map((w, i) => (
              <CardWindow key={w.key} label={w.label} w={w.window} now={now} tone={i === 0 ? 'accent' : 'slate'} />
            ))}
            {/* A block, not a paragraph: a marked cost's tip is one (OV-C229). */}
            <div className={styles.cardFoot}>
              <span>
                <MarkedCost text={c.cost} marks={c.costMarks} label={`${c.name}, spend`} />
              </span>
              <span>{c.read}</span>
              <span>{c.requests}</span>
            </div>
          </Tile>
        ))}
      </div>
    </Panel>
  );
}
