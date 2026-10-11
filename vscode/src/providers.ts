// Each provider's quota, as the status bar shows it (AGEN-0138): the router's summary part
// `providers`, one block per provider with an account, its serving account and its own
// windows. Pure, so `npm test` runs it under plain Node; `status.ts` puts it on screen.

/** One window as the router sends it: a fraction of the limit, and its reset in epoch seconds. */
export interface ProviderWindow { utilization?: number | null; resets_at?: number | null }

/** One provider's block of `GET /api/summary?parts=providers`. */
export interface ProviderQuota {
  provider: string;
  account?: { id?: string; label?: string | null; email?: string | null } | null;
  windows?: Record<string, ProviderWindow | null>;
  status?: string | null;
}

export interface ProvidersAnswer { providers?: ProviderQuota[] }

/** What the bar calls each provider, short: its models' name. */
const SHORT: Readonly<Record<string, string>> = { anthropic: 'Claude', kimi: 'Kimi', openai: 'OpenAI' };
/** Each provider's own name, for the tooltip. */
const NAME: Readonly<Record<string, string>> = { anthropic: 'Anthropic', kimi: 'Kimi Code', openai: 'OpenAI' };
/** Each window, short for the bar and whole for the tooltip, in the order they are shown. */
const WINDOWS: readonly (readonly [string, string, string])[] = [
  ['five_hour', '5h', '5-hour window'],
  ['seven_day', '7d', '7-day window'],
  ['month', 'mo', 'Month window'],
];

export const shortName = (provider: string): string => SHORT[provider] ?? provider;
export const providerName = (provider: string): string => NAME[provider] ?? provider;

const pct = (w?: ProviderWindow | null): string =>
  w?.utilization === null || w?.utilization === undefined ? '—' : `${Math.round(w.utilization * 100)}%`;

/** The windows a provider's block has, in order: Anthropic's 5-hour and 7-day, Kimi Code's
 *  5-hour and month. */
function windowsOf(p: ProviderQuota): (readonly [string, string, string, ProviderWindow | null | undefined])[] {
  const has = p.windows ?? {};
  return WINDOWS.filter(([key]) => key in has).map(([key, short, long]) => [key, short, long, has[key]] as const);
}

/** The bar's text: each provider by its short name, its windows after it. A router with one
 *  provider's accounts shows that one alone. */
export function barText(blocks: readonly ProviderQuota[]): string {
  return blocks
    .map((p) => `${shortName(p.provider)} ${windowsOf(p).map(([, short, , w]) => `${short} ${pct(w)}`).join(' · ')}`)
    .join('  ');
}

/** The tooltip's section for each provider: its name and serving account, then each window
 *  with its reset, in the viewer's own clock. */
export function tooltipSections(blocks: readonly ProviderQuota[], when: (epochSeconds: number) => string): { provider: string; title: string; lines: string[] }[] {
  return blocks.map((p) => {
    const who = p.account?.email || p.account?.label || p.account?.id;
    const lines = windowsOf(p).map(([, , long, w]) => `${long}: ${pct(w)}${w?.resets_at ? `, resets ${when(w.resets_at)}` : ''}`);
    if (p.status === 'rejected') lines.push('Refused: a window is spent until it resets.');
    return { provider: p.provider, title: who ? `${providerName(p.provider)} — ${who}` : providerName(p.provider), lines };
  });
}

/** How urgent the bar looks: `critical` when a provider refuses or a window is full,
 *  `warning` from 80%, else nothing. */
export function severityOf(blocks: readonly ProviderQuota[]): 'critical' | 'warning' | undefined {
  let worst: 'critical' | 'warning' | undefined;
  for (const p of blocks) {
    if (p.status === 'rejected') return 'critical';
    for (const [, , , w] of windowsOf(p)) {
      const u = w?.utilization;
      if (typeof u !== 'number') continue;
      if (u >= 1) return 'critical';
      if (u >= 0.8) worst = 'warning';
    }
  }
  return worst;
}

/** The blocks of an answer, when it is one: an older router refuses the part, and its
 *  answer then holds none. */
export function blocksOf(answer: unknown): ProviderQuota[] {
  const list = (answer as ProvidersAnswer | null | undefined)?.providers;
  return Array.isArray(list) ? list.filter((p) => p && typeof p.provider === 'string') : [];
}
