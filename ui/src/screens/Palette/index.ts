// Copied from Northstar ui/src/screens/Palette/index.ts at e9c1589.
// Changed: the types it names are agent-router's hits, steps and runs; Northstar's saved views are gone.

/// The palette's public face: the component the shell mounts and the hook that opens it.
/// A plain module rather than a component file, so each of the two lives in a module that
/// exports only its own kind and fast refresh can replace the component in place.
export { Palette } from './Palette';
export { usePaletteShortcut } from './shortcut';
export type { Context, Hit, Notice, Resolved, Run, Step } from './hits';
