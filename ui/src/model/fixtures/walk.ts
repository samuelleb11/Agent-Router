// New for agent-router: for the specs that ask "does anything here read 'undefined', 'NaN' or
// 'null'?" of a whole view, as the old checks asked it of the page's HTML. JSON.stringify would
// hide a NaN as `null`, so the view is walked instead.

/// Every place in `x` that would reach a reader as nonsense: text reading "undefined", "NaN",
/// "Infinity" or "null" where the router said no such thing, and a number that is not finite.
/// `said` is text the router itself sent, which may say anything and is left out of the test.
export function nonsense(x: unknown, said: readonly string[] = [], path = '$'): string[] {
  if (typeof x === 'string') {
    let own = x;
    for (const s of said) if (s) own = own.split(s).join('');
    return /undefined|NaN|Infinity|\bnull\b/.test(own) ? [`${path}: ${x}`] : [];
  }
  if (typeof x === 'number') return Number.isFinite(x) ? [] : [`${path}: ${x}`];
  if (Array.isArray(x)) return x.flatMap((v, i) => nonsense(v, said, `${path}[${i}]`));
  if (x && typeof x === 'object') return Object.entries(x).flatMap(([k, v]) => nonsense(v, said, `${path}.${k}`));
  return [];
}
