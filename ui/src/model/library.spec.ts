// New for agent-router: the Library model's rules that need no screen (docs/web-console-contracts.md,
// "Library").
import { describe, expect, it } from 'vitest';
import { bodyOf, diffLines, isName, itemParam, nameFrom, packFolder } from './library';

const bytes = (s: string) => new TextEncoder().encode(s);
const field = (tar: Uint8Array, at: number, len: number) => new TextDecoder().decode(tar.subarray(at, at + len)).replace(/\0.*$/s, '');

describe('the Library model', () => {
  it('LB-C9: a folder packs into a ustar the router reads — each file by its path inside the folder, clutter left out, sorted, checksummed', () => {
    const tar = packFolder([
      { path: 'bench/scripts/run.sh', bytes: bytes('echo hi\n') },
      { path: 'bench/SKILL.md', bytes: bytes('---\nname: bench\n---\n') },
      { path: 'bench/__pycache__/x.pyc', bytes: bytes('x') },
      { path: 'bench/._SKILL.md', bytes: bytes('apple double') },
    ]);
    expect(tar.length % 512).toBe(0);
    expect(field(tar, 0, 100)).toBe('SKILL.md');
    expect(field(tar, 257, 6)).toBe('ustar');
    expect(parseInt(field(tar, 124, 12), 8)).toBe(20);
    // The checksum is the sum of the header with its own field as spaces.
    const header = tar.slice(0, 512);
    const stored = parseInt(field(header, 148, 8).trim(), 8);
    header.fill(0x20, 148, 156);
    expect(stored).toBe(header.reduce((a, b) => a + b, 0));
    expect(field(tar, 1024, 100)).toBe('scripts/run.sh');
    expect(new TextDecoder().decode(tar)).not.toContain('__pycache__');
    expect(tar.subarray(tar.length - 1024).every((b) => b === 0)).toBe(true);
  });

  it('LB-C9: a name is offered from a folder or a file, and only one the Library takes is accepted', () => {
    expect(nameFrom('bench')).toBe('bench');
    expect(nameFrom('ship.md')).toBe('ship');
    expect(nameFrom('Vault.tar.gz')).toBe('vault');
    expect([isName('agent-router'), isName('Ship It'), isName('a--b'), isName('')]).toEqual([true, false, false, false]);
  });

  it('LB-C4, LB-C7: a file is shown without its frontmatter, and compared line by line', () => {
    expect(bodyOf('---\nname: x\n---\n\n# X\n')).toBe('# X\n');
    expect(bodyOf('# no frontmatter\n')).toBe('# no frontmatter\n');
    expect(diffLines('a\nb\nc', 'a\nc\nd')).toEqual([
      { kind: 'same', text: 'a' },
      { kind: 'removed', text: 'b' },
      { kind: 'same', text: 'c' },
      { kind: 'added', text: 'd' },
    ]);
    expect(itemParam('skill/bench')).toEqual({ kind: 'skill', name: 'bench' });
    expect(itemParam('plugin/x')).toBeNull();
  });
});
