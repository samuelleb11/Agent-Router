// New for agent-router: a project's initials, for its rail icon (~/components/InitialsIcon).

/// Up to two letters for a project's name: the first letter of each of its first two words, or
/// its first two letters when it is one word. `agent-router` is AR, `northstar` NO.
export function initials(name: string): string {
  const words = name
    .replace(/^[^\p{L}\p{N}]+/u, '')
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
  const first = words[0] ?? '';
  const second = words[1];
  const letters = second ? `${[...first][0] ?? ''}${[...second][0] ?? ''}` : [...first].slice(0, 2).join('');
  return letters.toUpperCase() || '?';
}
