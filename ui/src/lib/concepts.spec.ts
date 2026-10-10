import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import * as Icons from '@carbon/icons-react';
import { render } from '@testing-library/react';
import { createElement } from 'react';
import { describe, expect, it } from 'vitest';
import { AR_TOKENS, CONCEPTS, FAMILY_COLOUR, paint, type Concept } from './concepts';

/// The --ar-* tokens a theme's mixin declares in styles.scss: `ar-light` is Carbon white's set
/// and `ar-dark` g100's. The mixins hold declarations only, so the first closing brace ends them.
const styles = readFileSync(join(__dirname, '../styles.scss'), 'utf8').replace(/\/\/.*$/gm, '');
function declared(mixin: string): Set<string> {
  const body = new RegExp(`@mixin ${mixin} \\{([^}]*)\\}`).exec(styles);
  expect(body, `styles.scss has @mixin ${mixin}`).not.toBeNull();
  return new Set([...body![1]!.matchAll(/(--ar-[a-z0-9-]+)\s*:/g)].map((m) => m[1]!));
}
const THEMES = { white: declared('ar-light'), g100: declared('ar-dark') };

const entries = Object.entries(CONCEPTS) as [Concept, (typeof CONCEPTS)[Concept]][];
const carbon = Icons as unknown as Record<string, unknown>;

describe('concepts', () => {
  it('SH-C39: gives every concept exactly one icon, a real export of @carbon/icons-react', () => {
    for (const [concept, look] of entries) {
      // One icon: a single component, not a list and not a name to look up later.
      expect(Array.isArray(look.icon), concept).toBe(false);
      expect(['function', 'object'], concept).toContain(typeof look.icon);
      // The one @carbon/icons-react exports under that name, as installed.
      expect(carbon[look.iconName], `${concept}: @carbon/icons-react has no ${look.iconName}`).toBeDefined();
      expect(look.icon, concept).toBe(carbon[look.iconName]);
    }
  });

  it('SH-C39: never gives two concepts the same icon', () => {
    const byIcon = new Map<unknown, Concept[]>();
    for (const [concept, look] of entries) byIcon.set(look.icon, [...(byIcon.get(look.icon) ?? []), concept]);
    const shared = [...byIcon.values()].filter((concepts) => concepts.length > 1);
    expect(shared).toEqual([]);
    expect(new Set(entries.map(([, look]) => look.iconName)).size).toBe(entries.length);
  });

  it('gives every concept exactly one colour, an --ar-* token both themes declare', () => {
    for (const [concept, look] of entries) {
      expect(typeof look.colour, concept).toBe('string');
      expect(look.colour, concept).toMatch(/^--ar-[a-z0-9-]+$/);
      expect(THEMES.white.has(look.colour), `${concept}: ${look.colour} in white`).toBe(true);
      expect(THEMES.g100.has(look.colour), `${concept}: ${look.colour} in g100`).toBe(true);
    }
  });

  /// The plan's list, and the dashboard's sprite: the same concept wears the same icon on every
  /// screen, and a reader learns it once.
  it.each([
    ['project', 'Folder'],
    ['session', 'Terminal'],
    ['account', 'User'],
    ['quota', 'Meter'],
    ['subagents', 'Fork'],
    ['cache', 'Flash'],
    ['errors', 'WarningAlt'],
    ['putAway', 'Archive'],
    ['settings', 'Settings'],
    ['tools', 'Tools'],
    ['branch', 'Branch'],
    ['slow', 'Time'],
    ['scope', 'Filter'],
    ['search', 'Search'],
  ] as const)('OV-C4: %s wears %s', (concept, icon) => {
    expect(CONCEPTS[concept].iconName).toBe(icon);
    expect(CONCEPTS[concept].icon).toBe(carbon[icon]);
  });

  it('OV-C4: draws a project in the project colour and a session in the session colour', () => {
    expect(CONCEPTS.project.colour).toBe('--ar-project');
    expect(CONCEPTS.session.colour).toBe('--ar-session');
    expect(CONCEPTS.subagents.colour).toBe('--ar-subagent');
  });

  it('OV-C5: errors wear the error colour', () => {
    expect(CONCEPTS.errors.colour).toBe('--ar-bad-fill');
  });

  /// Requests waiting for their transcript are not failures, so their row must not wear the
  /// error triangle, greyed or not.
  it('OV-C6: gives "not yet attributed" a mark of its own, never the error icon or colour', () => {
    expect(CONCEPTS.unattributed.icon).not.toBe(CONCEPTS.errors.icon);
    expect(CONCEPTS.unattributed.colour).not.toBe(CONCEPTS.errors.colour);
  });

  /// The four parts of the context window keep their fixed colours, on the panel and in a
  /// request's detail alike.
  it('OV-C121: gives the context window its four parts, each in its own colour', () => {
    expect([CONCEPTS.tools, CONCEPTS.systemPrompt, CONCEPTS.conversation, CONCEPTS.toolResults].map((c) => [c.label, c.colour])).toEqual([
      ['tools (definitions)', '--ar-ctx-tools'],
      ['system prompt', '--ar-ctx-system'],
      ['conversation', '--ar-ctx-history'],
      ['tool results', '--ar-ctx-results'],
    ]);
  });

  /// A colour that means two things means neither. Identities never share a colour; a concept
  /// with no identity of its own wears the neutral slate and is told by its icon.
  it('OV-C8: never gives two identities one colour, and gives every other concept the neutral slate', () => {
    const identities = entries.filter(([, look]) => look.colour !== '--ar-tok');
    const colours = identities.map(([, look]) => look.colour);
    expect(new Set(colours).size).toBe(colours.length);
    expect(identities.map(([concept]) => concept).sort()).toEqual(
      ['cache', 'conversation', 'errors', 'project', 'session', 'subagents', 'systemPrompt', 'toolResults', 'tools'].sort(),
    );
  });

  it('keeps AR_TOKENS exactly the set each theme declares', () => {
    expect(new Set(AR_TOKENS)).toEqual(THEMES.white);
    expect(new Set(AR_TOKENS)).toEqual(THEMES.g100);
  });

  it('OV-C10: gives each model family its own colour, declared in both themes', () => {
    const colours = Object.values(FAMILY_COLOUR);
    expect(new Set(colours).size).toBe(4);
    for (const c of colours) {
      expect(THEMES.white.has(c)).toBe(true);
      expect(THEMES.g100.has(c)).toBe(true);
    }
  });

  /// An icon is drawn beside the word that says it; read out, it would be a shape with no word.
  it('draws every icon as an svg hidden from a screen reader', () => {
    for (const [concept, look] of entries) {
      const { container, unmount } = render(createElement(look.icon));
      const svg = container.querySelector('svg');
      expect(svg, concept).not.toBeNull();
      expect(svg, concept).toHaveAttribute('aria-hidden', 'true');
      unmount();
    }
  });

  it('paints a token as a CSS variable', () => {
    expect(paint('--ar-project')).toBe('var(--ar-project)');
  });
});
