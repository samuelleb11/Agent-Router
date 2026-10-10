// New for agent-router: a provider's own mark, beside its name or alone (SH-C56). The marks are the
// official files in ~/assets/providers, as published (see its README.md): Anthropic's has no colour
// of its own and is drawn in the text colour, through the file as a mask; Kimi's comes in a light
// and a dark file, and the one for the surface it sits on is shown. Neither is recoloured or
// reshaped, only sized to the text.
import { useTheme } from '@carbon/react';
import kimiOnLight from '~/assets/providers/kimi-code.svg';
import kimiOnDark from '~/assets/providers/kimi-code-dark.svg';
import { providerName, type Provider } from '~/model/settings';
import styles from './ProviderMark.module.scss';

/// The providers that have a mark. Any other is named in words only: nothing is drawn for it.
const MARKED = ['anthropic', 'kimi'] as const;
type Marked = (typeof MARKED)[number];
const marked = (p: string): p is Marked => (MARKED as readonly string[]).includes(p);

/// The mark itself, with no name: hidden from a screen reader, since whatever holds it names it.
/// Kimi's file follows the theme of the region it is drawn in, Carbon's `<Theme>` — the page's, or
/// the header's g100 — not only the page's.
function Glyph({ provider }: { provider: Marked }) {
  const { theme } = useTheme();
  const dark = theme === 'g90' || theme === 'g100';
  if (provider === 'anthropic') return <span className={styles.glyph} data-glyph="anthropic" aria-hidden="true" />;
  return <img className={styles.glyph} data-glyph="kimi" src={dark ? kimiOnDark : kimiOnLight} alt="" aria-hidden="true" draggable={false} />;
}

/// A provider, by its mark and its name (SH-C56).
///
/// - By default, the mark and the name: wherever there is room.
/// - `alone`, the mark alone, for a dense row: the name is its accessible name and, said again, its
///   tooltip, so a pointer that rests on it reads the same words a screen reader hears.
/// - `decorative`, the mark alone and hidden from a screen reader, for a row that names the provider
///   in words of its own beside it ("Kimi Code web", "acc-1 · Kimi Code"), which would otherwise be
///   heard twice.
/// - `words` stand beside the mark in place of the bare name, the name inside them: an account's
///   "active for Kimi Code" (OV-C231).
///
/// A provider with no mark is named in words in every form.
export function ProviderMark({
  provider,
  alone = false,
  decorative = false,
  words,
  className,
}: {
  /// `anthropic`, `kimi`; none is Anthropic's.
  provider: Provider | null | undefined;
  alone?: boolean;
  decorative?: boolean;
  words?: string;
  className?: string;
}) {
  // A row recorded before providers names none, and is Anthropic's.
  const p: Provider = provider || 'anthropic';
  const name = providerName(p);
  const cls = [styles.provider, className].filter(Boolean).join(' ');
  if (!marked(p)) {
    return (
      <span className={cls} data-provider={p}>
        {words ?? name}
      </span>
    );
  }
  if (decorative) {
    return (
      <span className={cls} data-provider={p} aria-hidden="true">
        <Glyph provider={p} />
      </span>
    );
  }
  if (alone) {
    return (
      <span className={cls} data-provider={p} role="img" aria-label={name} title={name}>
        <Glyph provider={p} />
      </span>
    );
  }
  return (
    <span className={cls} data-provider={p}>
      <Glyph provider={p} />
      <span className={styles.name}>{words ?? name}</span>
    </span>
  );
}
