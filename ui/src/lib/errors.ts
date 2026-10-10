// Copied from Northstar ui/src/lib/errors.ts at e9c1589.
// Changed: the router's refusal shapes, not Northstar's envelope. The router refuses with a non-2xx status, or with a 200 that carries `error` or `ok: false`, and its words are a plain string, so a refusal here is a status and a message.

/// What went wrong with a read or a write, in one of four kinds a reader acts on differently.
/// Unreachable and empty must never read as refused, and none of them as each other.
export type RefusalCode = 'refused' | 'unreachable' | 'empty' | 'unknown';

export interface RouterRefusal {
  readonly code: RefusalCode;
  /// The router's own words, verbatim: a paraphrase throws away the sentence that names the rule.
  readonly message: string;
  /// The HTTP status the refusal came with. A 200 for the refusals the router answers that way.
  readonly status?: number;
}

/// The router said no: a non-2xx answer, or a 200 whose body carries `error` or `ok: false`.
/// About a dozen of the router's endpoints refuse with a 200 (docs/web-console-carbon.md, "One
/// fetch layer"); read as success, a refused password reads "set".
export class Refused extends Error {
  override readonly name = 'Refused';
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

/// A 2xx that carried no body where one was promised. The router was reached and refused
/// nothing, and yet said nothing — a fact of its own, never worded as "could not reach".
export class EmptyBody extends Error {
  override readonly name = 'EmptyBody';
  constructor() {
    super('Reached the router, but the answer had no body.');
  }
}

/// No answer at all: the connection failed, or the router did not answer in time.
export class Unreachable extends Error {
  override readonly name = 'Unreachable';
  constructor(message = 'Could not reach the router.') {
    super(message);
  }
}

export const isEmptyBody = (err: unknown): err is EmptyBody =>
  err instanceof EmptyBody || (err instanceof Error && err.name === 'EmptyBody');

/// Read whatever a read or a write threw, without inventing anything. A `TypeError` is what a
/// fetch that never got a response throws, so it is unreachable too.
export function refusalOf(err: unknown): RouterRefusal {
  if (err instanceof Refused) return { code: 'refused', message: err.message, status: err.status };
  if (isEmptyBody(err)) return { code: 'empty', message: err.message };
  if (err instanceof Unreachable || err instanceof TypeError) return { code: 'unreachable', message: err instanceof Unreachable ? err.message : 'Could not reach the router.' };
  if (typeof err === 'string' && err.trim()) return { code: 'unknown', message: err };
  if (err instanceof Error && err.message) return { code: 'unknown', message: err.message };
  return { code: 'unreachable', message: 'Could not reach the router.' };
}

/// One line, in the router's words.
export function refusalText(err: unknown): string {
  return refusalOf(err).message;
}
