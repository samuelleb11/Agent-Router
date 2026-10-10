// New for agent-router: what the harness was asked, shared by its fetch and its event streams.

/// Every request the harness answered, and every one it could not, spelt `METHOD /path?query`
/// (a stream as `STREAM /path?query`), oldest first. `scripts/shoot.mjs` reads both and fails
/// on anything unanswered: a screenshot of a screen that quietly got nothing back would look
/// like a screen with nothing in it, and that is a different fact.
export interface Asked {
  readonly answered: string[];
  readonly unanswered: string[];
}

export const asked = (): Asked => ({ answered: [], unanswered: [] });
