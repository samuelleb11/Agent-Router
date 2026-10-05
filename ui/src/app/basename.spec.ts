// New for agent-router: where the console is mounted, read from the address it was opened at.
import { describe, expect, it } from 'vitest';
import { basenameFor } from './basename';

describe('basenameFor', () => {
  it('mounts the console at /next while it is served there, by the whole segment', () => {
    for (const path of ['/next', '/next/', '/next/live', '/next/settings/accounts']) expect(basenameFor(path), path).toBe('/next');
  });

  it('mounts it at the root anywhere else, a path that merely begins with the letters included', () => {
    for (const path of ['/', '/live', '/settings/router', '/nextcloud', '/nex']) expect(basenameFor(path), path).toBe('');
  });
});
