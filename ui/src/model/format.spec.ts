// @vitest-environment node
// New for agent-router: the formatters' contracts that the old checks never pinned, among them the
// ones the dashboard broke (docs/web-console-contracts.md, "Formatting").
import { describe, expect, it } from 'vitest';
import {
  ago,
  bucketName,
  client,
  compact,
  costNote,
  costSum,
  costWhy,
  count,
  duration,
  ESTIMATED_WHY,
  factTokens,
  factUsd,
  family,
  id8,
  kindRun,
  kindWord,
  ms,
  pct,
  plural,
  projectName,
  relative,
  requestCost,
  resetAt,
  rowDuration,
  share,
  shortModel,
  span,
  UNPRICED_WHY,
  usd,
} from './format';

const NOW = Date.UTC(2026, 9, 5, 12, 0, 0);

describe('format', () => {
  it('OV-C174: token quantities are compact, rounded before the next threshold is tested', () => {
    expect([0, 999, 2400, 120_000, 999_499, 999_600, 1_200_000, 2_680_000_000].map(compact)).toEqual(['0', '999', '2.4k', '120k', '999k', '1.00M', '1.20M', '2.68B']);
  });

  it('OV-C175: a token figure the router did not record reads "—" and only a recorded zero "0", where the page read "0"', () => {
    expect([compact(null), compact(undefined), compact(0)]).toEqual(['—', '—', '0']);
  });

  it('OV-C176: counts are exact, never compacted, and agree in number', () => {
    expect(count(1_234_567)).toBe((1_234_567).toLocaleString());
    expect(count(null)).toBe('—');
    expect([plural(1, 'session', 'sessions'), plural(2, 'session', 'sessions')]).toEqual(['1 session', '2 sessions']);
  });

  it('OV-C177: dollars, and a cost that could not be priced is never free', () => {
    expect([usd(1234.5), usd(4.567), usd(0.1234), usd(0.00123), usd(0), usd(null)]).toEqual([`$${(1235).toLocaleString()}`, '$4.57', '$0.123', '$0.0012', '$0', '—']);
  });

  it('OV-C229: a cost sum reads as its priced part, marked with how many requests it leaves out, the reason its tooltip; "$0" never stands for unknown', () => {
    expect(costSum({ cost: 3.1, unpriced: 4 })).toEqual({ text: '$3.10 + 4 unpriced', marks: [{ kind: 'unpriced', value: '4 unpriced', label: UNPRICED_WHY }] });
    expect(costSum({ cost: 1234.5, unpriced: 1200 }).text).toBe(`$${(1235).toLocaleString()} + ${(1200).toLocaleString()} unpriced`);
    // Nothing priced beside requests the router could not price is not free.
    expect(costSum({ cost: 0, unpriced: 4 }).text).toBe('not priced');
    expect(costSum({ cost: null, unpriced: 4 }).text).toBe('not priced');
    // With nothing left out: the dollars, "$0" for a zero the router measured and "—" for none given.
    expect([costSum({ cost: 3.1 }), costSum({ cost: 0, unpriced: 0 }), costSum({ cost: null })]).toEqual([
      { text: '$3.10', marks: [] },
      { text: '$0', marks: [] },
      { text: '—', marks: [] },
    ]);
    // One request: "not priced" with usage and no cost, "—" with no usage to price.
    expect(requestCost({ cost: null, prompt_tokens: 30_000 })).toEqual({ text: 'not priced', marks: [{ kind: 'unpriced', value: 'not priced', label: 'no rate for its model in the pricing table' }] });
    expect(requestCost({ cost: null, prompt_tokens: null, output_tokens: null }).text).toBe('—');
    expect(requestCost({ cost: 0.05, prompt_tokens: 30_000 })).toEqual({ text: '$0.050', marks: [] });
    // The reasons as one sentence, for a row's title, and said once under a list as text (SH-C30).
    expect(costWhy(costSum({ cost: 3.1, unpriced: 4 }))).toBe(`4 unpriced: ${UNPRICED_WHY}.`);
    expect(costWhy(costSum({ cost: 3.1 }))).toBeNull();
    expect(costNote([costSum({ cost: 3.1 }), costSum({ cost: 0, unpriced: 2 })])).toBe(`Unpriced: ${UNPRICED_WHY}.`);
    expect(costNote([costSum({ cost: 3.1 })])).toBeNull();
  });

  it('OV-C232: a cost sum that rests on estimated tokens reads "≈" before it, the reason its tooltip; nothing priced has no figure to mark', () => {
    expect(costSum({ cost: 3.1, estimated: 2 })).toEqual({ text: '≈$3.10', marks: [{ kind: 'estimated', value: '2 estimated', label: ESTIMATED_WHY }] });
    expect(costSum({ cost: 3.1, unpriced: 4, estimated: 2 })).toEqual({
      text: '≈$3.10 + 4 unpriced',
      marks: [
        { kind: 'unpriced', value: '4 unpriced', label: UNPRICED_WHY },
        { kind: 'estimated', value: '2 estimated', label: ESTIMATED_WHY },
      ],
    });
    expect(costSum({ cost: 0, unpriced: 4, estimated: 2 })).toEqual({ text: 'not priced', marks: [{ kind: 'unpriced', value: '4 unpriced', label: UNPRICED_WHY }] });
    expect(ESTIMATED_WHY).toMatch(/Kimi Code/);
    expect(costWhy(costSum({ cost: 3.1, unpriced: 4, estimated: 2 }))).toBe(`4 unpriced: ${UNPRICED_WHY}. 2 estimated: ${ESTIMATED_WHY}.`);
    expect(costNote([costSum({ cost: 3.1, estimated: 2 })])).toBe(`≈ estimated: ${ESTIMATED_WHY}.`);
  });

  it('OV-C179, OV-C61: a share has one decimal under 10 %, "—" with nothing to divide by, and a missing reading is never 0 %', () => {
    expect([pct(27, 8474), pct(99, 100), pct(0, 5), pct(5, 0)]).toEqual(['0.3%', '99%', '0%', '—']);
    // The page's `pct` read a missing utilisation as 0 %.
    expect(pct(null, 1)).toBe('—');
    expect([share(0.99), share(27 / 8474), share(0), share(null)]).toEqual(['99%', '0.3%', '0%', '—']);
  });

  it("OV-C180: a request's timing reads in milliseconds below a second and seconds above", () => {
    expect([ms(812), ms(3200), ms(null)]).toEqual(['812 ms', '3.2 s', '—']);
  });

  it('OV-C181: spans, times to come and times gone', () => {
    expect([45e3, 185e3, (3 * 60 + 12) * 60e3, 24 * 3600e3, -5000].map(duration)).toEqual(['45s', '3m 05s', '3h 12m', '1d 0h', '0s']);
    expect([relative(NOW + 5 * 60e3, NOW), relative(NOW + 130 * 60e3, NOW), relative(NOW + (3 * 24 + 4) * 3600e3, NOW), relative(NOW - 1, NOW)]).toEqual([
      'in 5 min',
      'in 2h 10m',
      'in 3d 4h',
      'now',
    ]);
    expect([ago(NOW - 30e3, NOW), ago(NOW - 3 * 60e3, NOW), ago(NOW - 2 * 3600e3, NOW), ago(NOW - 4 * 86400e3, NOW)]).toEqual(['just now', '3 min ago', '2 h ago', '4 d ago']);
    // After a week, the date.
    expect(ago(NOW - 10 * 86400e3, NOW)).not.toMatch(/ago/);
  });

  it('OV-C183: a reset instant in either spelling the router sends, and anything else unknown, never the epoch', () => {
    expect([resetAt('@1789416000'), resetAt('2026-10-05T12:00:00Z')]).toEqual([1_789_416_000_000, NOW]);
    expect([resetAt('soon'), resetAt('@'), resetAt('@x'), resetAt(null)]).toEqual([null, null, null, null]);
  });

  it('OV-C10, OV-C222: a model’s family is read from its name, Kimi Code’s ids a family of their own, and anything else other', () => {
    expect(['claude-opus-5-5', 'claude-sonnet-4-5', 'claude-haiku-4-5', 'opus[1m]'].map(family)).toEqual(['opus', 'sonnet', 'haiku', 'opus']);
    expect(['k3', 'k3-256k', 'k3[1m]', 'kimi-for-coding', 'kimi-for-coding-highspeed'].map(family)).toEqual(['kimi', 'kimi', 'kimi', 'kimi', 'kimi']);
    // A name that only starts like one is not one.
    expect(['k30', 'kimik2', 'gpt-x', null].map(family)).toEqual(['other', 'other', 'other', 'other']);
  });

  it("OV-C184: a model's short name is the phones', where the page also dropped the date and joined the version", () => {
    expect(['claude-opus-5-20260115', 'claude-opus-4-5-20251101', 'gpt-x'].map(shortModel)).toEqual(['opus-5-20260115', 'opus-4-5-20251101', 'gpt-x']);
    expect(shortModel(null)).toBe('—');
  });

  it('OV-C185: a bucket is named the same on every client, "15 minutes" where the page said "15 min"', () => {
    expect([60e3, 900e3, 3600e3, 6 * 3600e3, 86400e3, 30e3].map(bucketName)).toEqual(['minute', '15 minutes', 'hour', '6 hours', 'day', '30s']);
  });

  it('OV-C186, OV-C187: a project by the last part of its path, a session by eight characters, a client by its entrypoint', () => {
    expect([projectName('/code/agent-router/'), projectName('C:\\code\\win-app'), projectName(null)]).toEqual(['agent-router', 'win-app', '(unresolved)']);
    expect(id8('aaaaaaaa-1111-2222')).toBe('aaaaaaaa');
    // The VS Code extension launches the CLI, so its mark is tested first.
    expect([client('claude-vscode, claude-cli'), client('claude-cli'), client('sdk-ts'), client(null)]).toEqual(['VS Code', 'CLI', null, null]);
  });

  it("OV-C50, SE-C169: each kind has the reader's word, system prompt included, which the dashboard had none for", () => {
    expect(['text', 'tool_use', 'tool_result', 'thinking', 'system', 'other', 'constructor'].map(kindWord)).toEqual([
      'message',
      'tool call',
      'tool result',
      'thinking',
      'system prompt',
      'other',
      'constructor',
    ]);
  });

  it('OV-C50: a run of hits names its kind from the same table, and a kind the router adds later as it is sent', () => {
    expect(['text', 'tool_use', 'tool_result', 'thinking', 'system', 'image', 'constructor'].map(kindRun)).toEqual([
      'Messages',
      'Tool calls',
      'Tool results',
      'Thinking',
      'System prompts',
      'image',
      'constructor',
    ]);
  });

  it("SE-C16, SE-C84, SE-C134: the live view's own spellings", () => {
    expect([12e3, 300e3, 3 * 3600e3, 2 * 86400e3, -1].map(span)).toEqual(['12s', '5m', '3h', '2d', '0s']);
    expect([820, 2810, 18112, 180e3].map(rowDuration)).toEqual(['820ms', '2.8s', '18s', '3m']);
    expect([factUsd(0.1234), factUsd(4.561), factUsd(1234.4), factUsd(null)]).toEqual(['$0.123', '$4.56', `$${(1234).toLocaleString()}`, null]);
    expect([factTokens(1.2e9), factTokens(3.4e6), factTokens(56e3), factTokens(512), factTokens(null)]).toEqual(['1.2B', '3.4M', '56k', '512', '—']);
  });
});
