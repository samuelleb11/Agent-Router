// New for agent-router: the activity chart as Carbon Charts is handed it (docs/web-console-contracts.md,
// "Activity", "Refresh" and "Learned from Northstar's console"). The chart is a spy: @carbon/charts
// draws from SVG measurements jsdom does not have, and what matters here is what it is handed, when.
import { act, configure, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Timeline } from '~/data/overview';
import { ActivityChart } from './ActivityChart';
import { buckets } from './timeline';

/// The whole page is drawn in jsdom, beside every other spec in the run: generous limits, so a loaded
/// machine is not mistaken for a broken page. What "at once" means is asserted on the order of events,
/// never on how long they took.
vi.setConfig({ testTimeout: 30_000 });
configure({ asyncUtilTimeout: 10_000 });

const chart = vi.hoisted(() => ({ StackedBarChart: vi.fn() }));
vi.mock('@carbon/charts-react', () => ({
  StackedBarChart: (props: { data: unknown; options: unknown }) => {
    chart.StackedBarChart(props);
    return (
      <svg data-testid="chart">
        <rect className="chart-grid-backdrop" />
      </svg>
    );
  },
}));

const HOUR = 3_600_000;
const T0 = Date.UTC(2026, 9, 5, 9, 0, 0);
const timeline = (prompts: readonly number[], errors: readonly number[] = []): Timeline => ({
  start: T0,
  end: T0 + (prompts.length - 1) * HOUR,
  bucket_ms: HOUR,
  rows: prompts.map((p, i) => ({ t: T0 + i * HOUR, model: 'claude-opus-5', requests: 2, prompt_tokens: p, cost: 0.1, output_tokens: 10, cache_read_tokens: p / 2, subagent_tokens: 0, errors: errors[i] ?? 0 })),
});
const draw = (tl: Timeline) => <ActivityChart bk={buckets(tl, 0)} timeline={tl} now={T0 + 4 * HOUR} />;
const handed = () => chart.StackedBarChart.mock.calls.at(-1)![0] as { data: unknown; options: unknown };

let visibility: DocumentVisibilityState = 'visible';
beforeEach(() => {
  chart.StackedBarChart.mockClear();
  visibility = 'visible';
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
  // jsdom lays nothing out: the chart is given the box a browser would, its plot 600 px wide.
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const plot = this.classList?.contains('chart-grid-backdrop');
    return { left: plot ? 40 : 0, top: plot ? 10 : 0, width: plot ? 600 : 640, height: plot ? 180 : 220, right: plot ? 640 : 640, bottom: plot ? 190 : 220, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
  });
});
afterEach(() => {
  vi.restoreAllMocks();
});

const setVisible = (state: DocumentVisibilityState) =>
  act(() => {
    visibility = state;
    document.dispatchEvent(new Event('visibilitychange'));
  });

describe('the activity chart', () => {
  it('OV-C171: a poll that returns the same figures hands the chart nothing new, so nothing it shows is reset', async () => {
    const { rerender } = render(draw(timeline([100, 200, 300, 400])));
    await screen.findByTestId('chart');
    const first = handed();
    // A new snapshot, the same figures: new objects all the way down.
    rerender(draw(timeline([100, 200, 300, 400])));
    expect(handed().data).toBe(first.data);
    expect(handed().options).toBe(first.options);
    rerender(draw(timeline([100, 200, 300, 450])));
    expect(handed().data).not.toBe(first.data);
  });

  it('OV-C213: a chart is handed nothing while the tab is hidden, and the latest once when it is shown again', async () => {
    const { rerender } = render(draw(timeline([100, 200, 300, 400])));
    await screen.findByTestId('chart');
    const before = handed();
    await setVisible('hidden');
    rerender(draw(timeline([100, 200, 300, 999])));
    rerender(draw(timeline([100, 200, 300, 1_234])));
    expect(handed().data).toBe(before.data);
    const calls = chart.StackedBarChart.mock.calls.length;
    await setVisible('visible');
    expect(handed().data).not.toBe(before.data);
    expect((handed().data as { value: number }[]).at(-4)!.value).toBe(1_234);
    // Shown again, it is handed the latest once, and nothing more while the figures hold.
    const after = handed();
    rerender(draw(timeline([100, 200, 300, 1_234])));
    expect(handed().data).toBe(after.data);
    expect(handed().options).toBe(after.options);
    expect(chart.StackedBarChart.mock.calls.length).toBeGreaterThan(calls);
  });

  it('OV-C109: the chart’s own legend is off, so pressing it can never filter the chart', async () => {
    render(draw(timeline([100, 200])));
    await screen.findByTestId('chart');
    expect(handed().options).toMatchObject({ legend: { enabled: false, clickable: false }, tooltip: { enabled: false } });
  });

  it('OV-C115: a bucket with errors carries a mark above its bar, in the error colour', async () => {
    const { container } = render(draw(timeline([100, 200, 300, 400], [0, 2, 0, 1])));
    await screen.findByTestId('chart');
    await waitFor(() => expect(container.querySelectorAll('[data-error-mark]')).toHaveLength(2));
    const marks = [...container.querySelectorAll<HTMLElement>('[data-error-mark]')];
    expect(marks.map((m) => m.dataset.errorMark)).toEqual(['1', '3']);
    // Over its bucket's middle, above its bar's top: the second of four over a 600 px plot from 40 px,
    // its bar 200 of a 500 top.
    expect(marks[0]!.style.left).toBe(`${40 + 1.5 * 150}px`);
    expect(marks[0]!.style.top).toBe(`${10 + 180 * (1 - 200 / 500)}px`);
  });

  it('OV-C116: each bucket’s figures are reachable from the keyboard, from the latest back', async () => {
    render(draw(timeline([1_000, 2_000, 3_000, 4_000])));
    await screen.findByTestId('chart');
    const target = screen.getByRole('img', { name: /^Prompt tokens per hour, by model family: 4 buckets/ });
    act(() => target.focus());
    expect(screen.getByRole('tooltip')).toHaveTextContent('4.0k opus · 2 req');
    fireEvent.keyDown(target, { key: 'ArrowLeft' });
    expect(screen.getByRole('tooltip')).toHaveTextContent('3.0k opus · 2 req');
    expect(screen.getByRole('tooltip')).toHaveTextContent('50% from cache');
  });
});
