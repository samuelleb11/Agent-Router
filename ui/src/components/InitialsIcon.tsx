// Copied from Northstar ui/src/components/ShortnameIcon.tsx at e9c1589.
// Changed: a router project has no shortname, so its rail icon is the initials of its directory's name; sized with Carbon tokens rather than Northstar's hard-coded rem.
import { initials } from '~/lib/initials';
import styles from './InitialsIcon.module.scss';

/// A project's initials as its rail icon.
///
/// Collapsed, a rail of identical folder glyphs says nothing; the initials keep the collapsed
/// rail readable. The name itself is the link's text, so the initials are hidden from a screen
/// reader, which would otherwise hear each project announced twice.
export function InitialsIcon(name: string) {
  const text = initials(name);
  return function Icon() {
    return (
      <span className={styles.initials} aria-hidden="true">
        {text}
      </span>
    );
  };
}
