// New for agent-router: the one registry of what each concept looks like (docs/web-console-carbon.md,
// "Why Carbon, and what it does not cover"; contracts SH-C39, OV-C4).
import {
  Activity,
  Archive,
  Branch,
  Chat,
  Commit,
  DocumentConfiguration,
  Filter,
  Flash,
  Folder,
  Fork,
  Hourglass,
  Layers,
  List,
  MachineLearningModel,
  Meter,
  Return,
  Screen,
  Search,
  Settings,
  Terminal,
  Time,
  Tools,
  User,
  WarningAlt,
  type CarbonIconType,
} from '@carbon/icons-react';

/// The colours agent-router defines, exactly as `styles.scss` declares them in `ar-light` and
/// `ar-dark`. A concept's colour is one of these, so a misspelt token fails the type check rather
/// than drawing nothing; `concepts.spec.ts` holds this list to the stylesheet.
export const AR_TOKENS = [
  '--ar-ctx-tools',
  '--ar-ctx-system',
  '--ar-ctx-history',
  '--ar-ctx-results',
  '--ar-tok',
  '--ar-tok-2',
  '--ar-tok-3',
  '--ar-project',
  '--ar-session',
  '--ar-subagent',
  '--ar-model-opus',
  '--ar-model-sonnet',
  '--ar-model-haiku',
  '--ar-model-other',
  '--ar-ok',
  '--ar-warn',
  '--ar-bad',
  '--ar-ok-fill',
  '--ar-warn-fill',
  '--ar-bad-fill',
  '--ar-st-busy',
  '--ar-st-wait',
  '--ar-st-idle',
] as const;

export type ArToken = (typeof AR_TOKENS)[number];

/// A token as a CSS value, for a mark's `fill` or `background`. Never a `color`: text wears
/// Carbon's text tokens, and `scripts/check-palette.mjs` fails any other --ar-* used as text.
export const paint = (token: ArToken): string => `var(${token})`;

export interface ConceptLook {
  /// The reader's word for it, as a legend or a caption says it.
  readonly label: string;
  /// Its one icon. No two concepts share one.
  readonly icon: CarbonIconType;
  /// The icon's export name in @carbon/icons-react, which the spec checks it against.
  readonly iconName: string;
  /// Its one colour.
  readonly colour: ArToken;
}

/// A concept with no identity of its own wears the neutral slate, `--ar-tok`: hue is reserved for
/// identity (styles.scss), so slate is what says "no identity", and its icon is what tells the
/// concept. Identity colours go to the identities: project, session, subagent, the four parts of
/// the context window, cache (the darkest step of the read bar) and errors.
const look = (label: string, icon: CarbonIconType, iconName: string, colour: ArToken): ConceptLook => ({
  label,
  icon,
  iconName,
  colour,
});

/// Every concept the console draws, in one place, so the same concept wears the same icon and the
/// same colour on every screen. The first eight are the plan's; the rest are the dashboard's
/// sprite (assets/index.html, `g-*`), each given a Carbon icon of its own.
export const CONCEPTS = {
  project: look('project', Folder, 'Folder', '--ar-project'),
  session: look('session', Terminal, 'Terminal', '--ar-session'),
  account: look('account', User, 'User', '--ar-tok'),
  quota: look('quota', Meter, 'Meter', '--ar-tok'),
  subagents: look('subagents', Fork, 'Fork', '--ar-subagent'),
  cache: look('cache', Flash, 'Flash', '--ar-tok-3'),
  errors: look('errors', WarningAlt, 'WarningAlt', '--ar-bad-fill'),
  putAway: look('put away', Archive, 'Archive', '--ar-tok'),

  mainThread: look('main thread', Commit, 'Commit', '--ar-tok'),
  tools: look('tools (definitions)', Tools, 'Tools', '--ar-ctx-tools'),
  systemPrompt: look('system prompt', DocumentConfiguration, 'DocumentConfiguration', '--ar-ctx-system'),
  conversation: look('conversation', Chat, 'Chat', '--ar-ctx-history'),
  toolResults: look('tool results', Return, 'Return', '--ar-ctx-results'),
  contextWindow: look('context window', Layers, 'Layers', '--ar-tok'),
  model: look('model', MachineLearningModel, 'MachineLearningModel', '--ar-tok'),
  activity: look('activity', Activity, 'Activity', '--ar-tok'),
  requests: look('requests', List, 'List', '--ar-tok'),
  search: look('search', Search, 'Search', '--ar-tok'),
  branch: look('git branch', Branch, 'Branch', '--ar-tok'),
  slow: look('slow', Time, 'Time', '--ar-tok'),
  scope: look('scope', Filter, 'Filter', '--ar-tok'),
  settings: look('settings', Settings, 'Settings', '--ar-tok'),
  /// Requests waiting for their transcript (OV-C6): not a failure, so never the error triangle.
  unattributed: look('not yet attributed', Hourglass, 'Hourglass', '--ar-tok'),
  /// The machine a hosted router heard a session from (SE-C219, SE-C221): a computer, as both
  /// phones draw it.
  machine: look('machine', Screen, 'Screen', '--ar-tok'),
} as const satisfies Record<string, ConceptLook>;

export type Concept = keyof typeof CONCEPTS;

/// A model's family colour (OV-C10). The family itself is read from the model's name by the
/// formatters; "other" is drawn as a hollow ring, never as a fourth fill.
export const FAMILY_COLOUR = {
  opus: '--ar-model-opus',
  sonnet: '--ar-model-sonnet',
  haiku: '--ar-model-haiku',
  other: '--ar-model-other',
} as const satisfies Record<string, ArToken>;

export type Family = keyof typeof FAMILY_COLOUR;
