// Copied from Northstar ui/src/app/notify.ts at e9c1589.
// Changed: nothing.

/// The notice store, and the one way to add to it.
///
/// Apart from the component that draws them so that a module exporting `notify` exports
/// nothing else: a file that mixes a component with a plain value cannot be hot-replaced
/// in place, and every screen in the console imports `notify`.
export interface Notice {
  readonly id: number;
  readonly kind: 'success' | 'error' | 'info' | 'warning';
  readonly title: string;
  readonly detail?: string;
}

let seq = 0;
let notices: readonly Notice[] = [];
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

function push(kind: Notice['kind'], title: string, detail?: string, ttl = 5000) {
  const id = ++seq;
  notices = [...notices, { id, kind, title, detail }];
  emit();
  setTimeout(() => dismiss(id), ttl);
}

/// Read by the component that draws the notices, and by nothing else.
export function dismiss(id: number): void {
  notices = notices.filter((n) => n.id !== id);
  emit();
}

export function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export const snapshot = (): readonly Notice[] => notices;

/// Short, unobtrusive confirmation that a write landed — and a louder one when it did not.
/// Refusals that belong beside a control are rendered THERE, not here; this is for the
/// outcome of an action whose control is no longer on screen.
export const notify = {
  saved: (what: string) => push('success', what),
  refused: (what: string, why: string) => push('error', what, why, 8000),
  note: (what: string, detail?: string) => push('info', what, detail, 6000),
  warn: (what: string, detail?: string) => push('warning', what, detail, 8000),
};
