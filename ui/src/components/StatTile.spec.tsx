// Copied from Northstar ui/src/components/StatTile.spec.tsx at e9c1589.
// Changed: the router's tones (ok, warn, bad) in place of Northstar's buckets; a value not given is an em dash with no tone of its own; the tile holds what is drawn under its lines; and the largest tile spans its row on a phone.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { StatTile } from './StatTile';

const plain = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
const scss = plain(readFileSync(join(__dirname, 'StatTile.module.scss'), 'utf8'));

/// A pointer with no hover, for the rule that the meaning is text at every width.
function pointer(coarse: boolean) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: coarse && query === '(pointer: coarse)',
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }));
}

afterEach(() => vi.unstubAllGlobals());

describe('StatTile', () => {
  it('shows its label, its value and one detail line', () => {
    const { container, getByText } = render(<StatTile label="Requests" value="412" detail="2 sessions · 2 projects" />);
    expect(getByText('Requests')).toBeInTheDocument();
    expect(getByText('412')).toBeInTheDocument();
    expect(getByText('2 sessions · 2 projects')).toBeInTheDocument();
    expect(container.firstElementChild).not.toHaveAttribute('data-tone');
  });

  /// Tone is a meaning: the router's three statuses, the figure in the status text token and the
  /// stripe in its fill.
  it.each([['ok'], ['warn'], ['bad']] as const)('carries the %s tone in the status tokens', (tone) => {
    const { container } = render(<StatTile label="Errors" value="3" tone={tone} />);
    expect(container.firstElementChild).toHaveAttribute('data-tone', tone);
    expect(scss).toMatch(new RegExp(`\\.tone_${tone} \\{\\s*--tone: var\\(--ar-${tone}\\);\\s*border-inline-start-color: var\\(--ar-${tone}-fill\\);`));
  });

  /// A value the router did not give is an em dash, never a zero, and it says so to a
  /// screen reader; on the Overview that is often a fact (nothing priced, no latency yet) rather
  /// than a failed read, so it takes no warning tone of its own.
  it('renders a value not given as an em dash, never a zero', () => {
    const { container, queryByText, getByLabelText } = render(<StatTile label="Spend" value={null} detail="nothing priced in scope" />);
    expect(container.textContent).toContain('—');
    expect(queryByText('0')).toBeNull();
    expect(getByLabelText('not given')).toHaveTextContent('—');
    expect(container.firstElementChild).toHaveAttribute('data-withheld', 'true');
    expect(container.firstElementChild).not.toHaveAttribute('data-tone');
  });

  /// A zero the router counted is a fact and stays a zero.
  it('keeps a real zero', () => {
    const { container, getByText } = render(<StatTile label="Errors" value="0" />);
    expect(getByText('0')).toBeInTheDocument();
    expect(container.firstElementChild).not.toHaveAttribute('data-withheld');
  });

  /// The meaning is text at both widths, and in a tooltip at neither.
  it('prints its meaning at both widths and hides it in no tooltip', () => {
    for (const coarse of [false, true]) {
      pointer(coarse);
      const r = render(<StatTile label="Errors" value="3" meaning="A status of 400 or more, or an error recorded." />);
      expect(r.getByText('A status of 400 or more, or an error recorded.')).toBeInTheDocument();
      expect(r.container.querySelector('[title]')).toBeNull();
      r.unmount();
    }
  });

  it('draws what it holds under its lines: a sparkline, a meter, an action', () => {
    const { getByRole } = render(
      <StatTile label="Errors" value="3">
        <button type="button">List the 3 errors</button>
      </StatTile>,
    );
    expect(getByRole('button', { name: 'List the 3 errors' })).toBeInTheDocument();
  });

  /// On a phone the tiles sit two to a row and the largest takes the row; the shot script measures
  /// the layout itself in a browser.
  it('spans its row on a phone when it is the largest', () => {
    const { container } = render(<StatTile label="Prompt tokens read" value="12.40M" hero />);
    expect(container.firstElementChild?.className).toMatch(/hero/);
    expect(scss).toMatch(/@include breakpoint-down\('md'\) \{\s*\.hero \{\s*grid-column: 1 \/ -1;/);
  });
});
