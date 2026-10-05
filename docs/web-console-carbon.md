# Moving the web UI to React and IBM Carbon

2026-10-05. An assessment and a plan; nothing in it is built yet. It covers both pages the
router serves today — the dashboard (`assets/index.html`, 2,568 lines, at `/`) and the live
view (`assets/live.html`, 1,676 lines, at `/live`) — and what it takes to rewrite them as one
responsive React app on IBM Carbon, the way Northstar's console was rewritten in September
(Northstar's ADR 0015, `docs/adr/0015-carbon-react-is-the-console.md` in that repository), keeping
agent-router's colour coding. Eight readers inventoried the two pages slice by slice, and two
more read the serving, distribution and test surface and Northstar's console. Their raw
reports lived in a scratch directory that a reboot removed; what is here comes from their
summaries and from spot checks against the code. Findings that only a reader observed are
listed under [Unverified](#unverified). Line numbers refer to commit `5e3621b`.

## In one paragraph

It is feasible, and the order matters more than the framework. The shell is Northstar's:
its topbar, ⌘K search and command palette, rail and phone drawer, theme switch and Settings
pages. Carbon supplies the forms, dialogs, toggles, tags, notifications, lists, tabs, code
snippets and two of the charts. Agent-router's colour coding survives as a short list of custom tokens layered on
Carbon's theme, the way Northstar's status colours do. About a dozen small visualisations
Carbon does not have (meters, share bars, bar rows, sparklines, the burn-down, status dots,
chat bubbles) become custom components drawn with Carbon tokens. The build gains Node and
Vite, but the product stays one binary: the bundle is embedded with `rust-embed` and served
behind the same guard. The two pages carry more than 500 behavioural contracts, about 75 of
them bugs that once shipped, and the iPhone and Android apps copy the live view's behaviour,
so the first deliverable is not a component but those contracts, ported from the two check
scripts into Vitest and run in CI. The rewrite also fixes, by construction, defects the
readers found. One of them, a script injection in today's live view, should be fixed now
whatever is decided about the rest.

## What exists today

### Two pages, two palettes

| | Dashboard | Live view |
| --- | --- | --- |
| File, size | `assets/index.html`, 2,568 lines (352 of CSS) | `assets/live.html`, 1,676 lines (364 of CSS) |
| Served | `/`, `include_str!` at `src/api.rs:23` | `/live`, `include_str!` at `src/live.rs:1202` |
| Headers | `X-Frame-Options: DENY`; CSP `frame-ancestors 'none'; base-uri 'none'; form-action 'none'` (`api.rs:25-39`) | `Cache-Control: no-store`, nothing else (`live.rs:1204-1214`) |
| Palette | light-first, indigo accent, zinc neutrals, checked with the dataviz validator | dark-first, blue accent, `--st-*` state colours shared with both phone apps |
| DOM | built with `textContent` | `innerHTML` through `esc()` |
| Refresh | one 5 s poll of eight endpoints read as one snapshot | SSE per open session, a 10 s silence watchdog, polled prompts |

The two pages share no helpers and use the same token names with different values (accent,
ok, warn). The [product assessment](product-assessment-2026-09-13.md) (item 3) already asked
for one web app with one shell, the shape the iPhone app has.

### What the readers counted

| Page | Features | Contracts | Of which past bugs |
| --- | ---: | ---: | ---: |
| Dashboard | 152 | 326–340 | 27–28 |
| Live view | 74 | 182–187 | 48–50 |
| Both | 226 | 508–527 | 75–78 |

The ranges are two reading passes over the same slices; the features matched exactly. A
contract is a rule a rewrite has to keep: "red only for a crossing that survives a quiet
week", "a replay replaces the feed only on a fresh connection's first event", "quota
percentages are never summed across accounts". Most come from the pages' own comments, which
record why each rule exists.

### Who opens them

Every external consumer opens a bare `/` or `/live`:

- The VS Code extension opens both in Simple Browser, with an iframe-webview fallback
  (`vscode/src/dashboard.ts:8-24`), and the status bar opens the dashboard.
- The menubar app opens `http://127.0.0.1:<port>` (`menubar/src/main.rs:253-256`).
- The iPhone's "Open the dashboard in Safari" opens `http://host:port`
  (`ios/AgentRouter/Sources/Views/RouterSettingsView.swift:146-148`).
- Push payloads carry no URL; the phones route a tap natively by session id.

So `/` and `/live` must keep answering, plus the state formats the pages write themselves, for
bookmarks: `#a=&p=&s=&r=&o=` on the dashboard and `?session=` on the live view.

### The phones follow the live view

[Session liveness](session-liveness.md) (lines 447-475) makes the web page the reference for
iOS and Android, and both apps pin it in tests named after it: on iOS
`testSessionsGroupLikeTheWebView`, `testMarkdownBlocksMatchTheWebView`,
`testTheProjectDotRanksTheSameEverywhere`,
`testSilenceMeansTenSecondsAndAReconnectRebuildsTheFeed` and four more in `LiveTests.swift`,
plus `DashboardTests` and `ProjectionTests`; on Android seven in `LiveTest.kt` and two in
`SearchTest.kt`. The session model (tone, what a row says it is doing, grouping, the project
dot's ranking, ten seconds of silence, which form an answer goes to) and the Markdown blocks
are therefore three-client contracts. The rewrite keeps them exactly or changes all three
clients. One reader found the web `md()` and the phones' `Markdown.blocks` already disagree on
numbered lists, joined paragraphs and quotes, unterminated fences and heading levels.

### The checks

`scripts/check-dashboard-ui.js` holds 17 checks and `scripts/check-live-ui.js` 28; on
2026-10-05 they passed 17/17 and 27/27 (the captured-stream check is skipped without a file).
**Nothing runs them**: no CI job, no Makefile, no hook, no mention in the README. Both cut the
inline `<script>` out of the HTML and drive it against a fake DOM, so they die with the pages.
Their assertions and fixtures (`scripts/fixtures/forecast.json` and the synthetic stream at
`check-live-ui.js:924-955`) are what gets ported.

### What a literal port would inherit

- **Phones scroll sideways** on the dashboard: by 143, 128 and 89 px at 360, 375 and 414 px
  wide. The causes are bar-row meta lines that never wrap, the session chips, the request
  row's 11-track grid and a 330 px two-up minimum.
- **Facts that live only in hover tooltips or vanish at 700 px and below**: the spend split,
  p95 latencies, per-bucket values, reset times, context costs and character counts, the
  project path, the account id. A phone and a keyboard cannot reach them.
- **Too small for a finger**: fields of 12–15 px (iOS zooms into anything under 16) and
  targets of 26–37 px.
- **Contrast failures**: the dark-mode primary button at about 3:1, white labels inside dark
  bars at 2.56:1, white on the live view's dark user bubble at 2.52:1, light "faint" text at
  2.7–3.1:1, light error text at 3.2:1.
- **Faults of the hand-rolled architecture**: a scope change during a refresh lets the older
  reply paint last; the "older" and "archived" folds snap shut on every 5 s rebuild, along with
  focus and sideways scroll; async results land in whichever session is open now.

## Why Carbon, and what it does not cover

Northstar's reasons apply. Its old console had 2,348 lines of ad-hoc CSS and no tokens, and
the density and alignment faults came from that, not from a want of polish. Agent-router's
pages hold 716 lines of hand-written CSS in two dialects. Carbon brings an 8 px spacing
scale, a type scale, motion tokens, light and dark themes and accessible components, and
Northstar has already paid for learning its traps.

What Carbon does not have, and agent-router needs, becomes small components drawn with Carbon
tokens and agent-router's colours, each with a spec:

| Component | Why custom |
| --- | --- |
| `StatusDot`, `StatusPill` | Carbon's `IconIndicator` and `ShapeIndicator` are preview exports in Carbon's own status colours, with no hollow ring, halo or pulse. The phones share the `--st-*` tones. |
| `UsageMeter` | `ProgressBar` is `role=progressbar` with an `aria-live` helper that would re-announce on every poll; the quota needs `role=meter` and the 70 % / 90 % thresholds. |
| `ShareBar` + legend | The row-to-segment highlight and the 24 px hover zone for thin segments; `MeterChart` loses both. |
| `BarRow`, `BarList` | Selectable rows (`aria-pressed`) that scope the page, with a meta line and a subagent overlay. `ContainedListItem` puts extra props on its `<li>`, not its button. |
| `Sparkline` | The 68 px strip; a Carbon `LineChart` needs 100–120 px and cannot mark only the last point. |
| `BurnDown` | The dashed projection ray, the single crossing marker and the quiet-to-busy cone carry the chart's meaning; Carbon Charts draws none of them. |
| `DataTip` | A data tooltip that a tap can pin, so tooltip facts reach a phone. |
| `ScopePicker`, `StickyScopeBar` | Rows with a name, a left-ellipsised path and a money meta, filterable; `MenuItem` labels are strings only. |
| `RequestRow` | A phone-first card with an expandable detail; 300 of them rule out `ExpandableTile`, which runs a `ResizeObserver` each. |
| Chat bubble, rail, `Clamp`, `FactsStrip`, `Markdown` | `@carbon/react` has no chat primitives; Markdown renders to React elements, never HTML. |

One `concepts.ts` registry gives each concept its icon and colour, so the same concept wears
the same icon on every screen: Folder for a project, Terminal for a session, User for an
account, Meter for quota, Fork for subagents, Flash for cache, WarningAlt for errors, Archive
for put away. All of these exist in `@carbon/icons-react`.

## Colour coding and tokens

The rule Northstar follows, applied here: **Carbon's theme owns the neutrals, type, spacing,
motion and interactive chrome; agent-router defines only colours that mean something.**

| Today | Meaning | In the Carbon app |
| --- | --- | --- |
| `--bg`, `--panel`, `--ink`, `--muted`, `--line` (dashboard); `--fg`, `--dim`… (live) | page, panel, text, secondary text, rules | Carbon `$background`, `$layer-01`, `$text-primary`, `$text-secondary`, `$border-subtle-01` |
| `--accent` (indigo on the dashboard, blue on the live view) | interactive chrome | **Decision A** |
| `--ok`, `--warn`, `--bad` | status | `--ar-ok`, `--ar-warn`, `--ar-bad`, two tokens each: one safe for text, one for fills. Carbon's support tokens fail 4.5:1 as text; Northstar measured 2.3:1 and 1.2:1. |
| `--c1`–`--c4` | context window: tools, system, history, tool results | kept |
| `--tok`, `--tok-2`, `--tok-3` | token magnitude ramp | kept |
| `--proj`, `--sess`, `--sub` | project, session, subagent identity | kept |
| `--m-opus`, `--m-sonnet`, `--m-haiku`, `--m-other` | model family; also colours the activity chart | kept |
| `--st-*` (live) | session state, shared with both phones | kept, values unchanged |

Three pieces of work come with keeping them:

1. **One set, not two.** Where the pages disagree (accent, ok, warn), the dashboard's
   validated values win for data. `--st-*` stays as the phones have it.
2. **Four collisions to settle.** The light accent equals `--c1` (tools), subagent pink equals
   tool-results pink and both appear in one request's detail, session purple equals Opus
   purple, and `--proj` equals the muted text colour.
3. **Re-validate on Carbon's surfaces.** The palette was checked against today's panels. On
   Carbon g100's `$layer-01` the dark `--c1` falls from 4.31:1 to 3.65:1. The CSS says to re-run
   the dataviz validator rather than adjust by eye, and every contrast failure above is fixed in
   the same pass.

The theme is Northstar's: Carbon **white** for light and **g100** for dark, with the topbar
g100 in both. The interactive chrome is Carbon's blue, as in Northstar. A system/light/dark
choice is remembered in the browser, using Northstar's `theme.ts` as it stands. Indigo stays
the tools colour, which ends today's accent-equals-tools collision. The type is IBM Plex Sans and Mono, self-hosted (Northstar's four faces, 252 kB).
Carbon's 12 px floor enlarges today's 9.5–11.5 px labels, and Carbon's square corners replace
the 10 px radius.

## Architecture

### One app in Northstar's shell

| Route | Today | Becomes |
| --- | --- | --- |
| `/` | the dashboard | **Overview** |
| `/live`, `/live?session=` | the live view | **Sessions** |
| `/settings/<section>` | a modal over the dashboard | **Settings**, pages under one header with tabs: Accounts, Claude Code, Network, Password, Notifications, Router |

The shell is Northstar's (`ui/src/app/Shell.tsx`, 234 lines), reused rather than redesigned:

- **Topbar.** Always g100. It holds the hamburger, the name, the freshness badge (Northstar's
  `LiveBadge`, standing in for today's live/stale/unreachable chip), a search action that opens
  the ⌘K palette, and the system/light/dark switch.
- **Navigation.**
  - On a desk, a rail that collapses and is remembered: Overview, Sessions, then the projects,
    each opening the Overview scoped to it, where Northstar lists its projects. Settings is
    pinned at the foot.
  - On a phone the same links sit in a drawer behind the hamburger, closed by the overlay,
    Escape or arriving.
- **The palette.** A passive modal near the top on a desk, and a full-height sheet under the
  topbar on a phone. Here it searches the router's conversation index (`/api/search`), jumps
  to a project or session, and runs commands behind `>`: switch account, set a session's
  model, effort or mode, archive, theme.
- **Settings.** Northstar's `screens/Settings/Layout.tsx`: one page header, with the sections
  as URL-driven tabs that are real links.
- **Identity.** Carbon's 48 px header cannot wrap the way today's does (118 px tall at
  375 px). The serving account's identity moves into the quota caption and the Accounts
  settings.
- **The live view** keeps its 100 dvh layout with safe-area insets and the `visualViewport`
  keyboard pin, inside the shell's content area. Northstar has no precedent for these, so the
  shell's fixed header needs the safe-area padding added.

The code is copied from Northstar, with the source commit noted at the top of each copied
file. A shared package would be worth extracting only if a third console appears; until then,
two copies are cheaper than a published dependency.

**URL state.** Scope, time range, request filters and Find live in the query string. A scope
change pushes history, so Back un-scopes (Northstar's rule); today the dashboard replaces it.
An old `#a=&p=…` link is read once and rewritten, so bookmarks keep working.

### Data

- **One fetch layer** that treats a 200 carrying `error`, or `ok:false`, as a failure. About
  a dozen endpoints answer refusals that way: quota, the usage token, sign-in start, the
  password, the network switch, the setup and wrapper switches, and push configuration and
  test. Today's `post()` only throws on a non-2xx, so a refused password reads "set"
  (`index.html:810-815`, `2257`).
- **Hand-written TypeScript types** for the responses, taken from the Rust handlers. Northstar
  generates its client from an OpenAPI spec; agent-router has none, and the types are the
  smaller job.
- **The dashboard poll** is one TanStack Query keyed by scope, reading the eight endpoints as
  one snapshot. `api.rs:300-303` promises that every figure on the page agrees, and the
  "not yet attributed" row depends on it. It refetches 5 s after the previous read completes,
  keeps the previous data on a scope change, and turns consecutive failed cycles (not HTTP
  attempts) into the stale and unreachable states. It never retries quietly. Quota and forecast
  keep their last good value. Auth and accounts poll every 30 s, and push is read when Settings
  opens. Every request gets a timeout, which today's code lacks.
- **The live stream** is a `useSessionStream` hook built on `EventSource`. It keeps a generation
  guard against replaced connections, the 10 s watchdog, replace-only-on-the-first-event, and
  the clock-set-back guard, and survives React StrictMode's double mount. It reads an error
  during CONNECTING as reconnecting and registers every named event. Every async result is
  tagged with its session id and dropped if another session is open by then. The Sessions
  screen is keyed on the session id, so a draft or a half-answered form never follows the
  reader to another session.
- **Pure modules, no React**: the session model, the Markdown parser, the formatters the iPhone
  app copies by name, the forecast's wording rules and the scope model. These are what the
  contracts test first.

### Serving and security

- **Embedding.** `rust-embed` over `ui/dist` with `allow_missing`. Explicit routes go *above*
  the guard layer at `main.rs:383`, or they would skip it:
  - `/`, `/live` and `/settings` serve `index.html` with `no-store`.
  - `/assets/{*path}` serves hashed files `immutable`, and an unknown file is a plain 404.
  - `/favicon.svg` stays.
  - Never a fallback or a wildcard: the fallback is the proxy, and a wildcard would catch
    `/v1/*` and break Claude Code.
- **Headers.**
  - `X-Content-Type-Options: nosniff` everywhere; the router sends it nowhere today.
  - For the document: `default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self';
    img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none';
    form-action 'none'` plus the frame policy.
  - React and Carbon set inline styles through the CSSOM, which CSP does not govern. Whether
    any dependency injects a `<style>` element is checked on the first build, with the policy
    in Report-Only mode until it is.
- **Framing (Decision D).** One document now serves every route, so one policy covers both
  pages. Today the dashboard refuses all framing, which by the headers' logic breaks VS Code's
  Simple Browser and its fallback on editors before 1.109. The live view allows framing from
  anywhere. It is the page that approves prompts and can switch a session to
  bypass-permissions mode, and a web page can frame `http://127.0.0.1:8787/live`, because the
  guard's Host and peer checks both pass for it. The proposal is `frame-ancestors` limited to
  VS Code's own schemes (`vscode-webview:` and `vscode-file:`), which no web page can use, with
  `X-Frame-Options` dropped because it cannot express an allow-list. This is to be checked in
  VS Code before it ships.
- **Transcript text is never HTML.** Markdown and every string from a transcript render as
  React text. That closes the injection below by construction.
- **Handover.** A tab left open across a deploy asks for chunks the new binary does not have.
  The app reloads on `vite:preloadError`, and lazy chunks are kept to the charts.
- **Development.** The Vite dev server stays on loopback and proxies `/api` to a scratch router
  on a free port, never 8787. If it runs on a copy of the store, scrub the copy first: it
  carries the stored bind, the password, the wrapper switch and the registered phones. It must not
  listen on the LAN: the guard trusts the connecting peer, so every LAN browser would look
  local and skip the password. Northstar solves this by forwarding `X-Forwarded-For` to a
  server that honours it, and agent-router's does not.

### Build and distribution

- **`ui/`** at the repository root, set up as Northstar's is: pnpm, Vite, TypeScript, Vitest,
  ESLint and the notices script. `vscode/` stays on npm, so the repository has two package
  managers. Northstar's scripts assume pnpm, and copying them unchanged is worth that.
- **Missing-UI policy in `build.rs`.** The hosting work is adding a `build.rs` for the build
  id, and this extends it. A debug build with no `ui/dist` serves a placeholder page naming the
  command and prints a `cargo:warning`. A release build without it is an error unless
  `AGENT_ROUTER_ALLOW_NO_UI=1` is set. It also adds `rerun-if-changed=ui/dist`.
- **`scripts/build-target.sh`** builds the UI once before the cargo step. Targets build in
  parallel with separate `CARGO_TARGET_DIR`s, so a UI build per target would race on `ui/dist`.
- **CI.** A `build:ui` job on `node:22` produces `ui/dist` as an artifact, and both binary jobs
  take it through `needs`, so the Mac runner needs no pnpm. A UI check job (lint, typecheck,
  tests, notices, chunk budget) sits beside the Rust checks the hosting work is adding. The
  ported contract tests run there.
- **Docker** (the hosting work's image): a `node:24-bookworm-slim` console stage before
  `cargo build`, copying `ui/dist` in. That session has marked the spot.
- **What the documents promise.** `README.md:108-110` says "no build step, no node toolchain,
  no network", and `ARCHITECTURE.md:421` says "no build step and no node toolchain, which keeps
  the one static binary promise intact". The binary and the no-network promise survive. The
  no-Node promise does not: building from source will need Node for the console.
- **Versioning.** `--ensure` compares only `CARGO_PKG_VERSION`, so the extension will not
  replace a running router for a UI-only change unless the version is bumped. That is already
  true today.

### Carbon traps already known

From Carbon 1.116's source and Northstar's history, before writing a line:

- **Selective SCSS partials fail silently.** A component whose partial is missing renders as
  unstyled text, and `scss/layout` is what defines the height tokens every sized component
  reads.
- **`Search` swallows Escape**, clearing the box and stopping propagation. Find's "Escape
  closes" needs `onKeyDown`.
- **`ActionableNotification` is an alertdialog that takes focus.** An Enter meant for the
  composer could approve a prompt, so the permission card is a `Callout`.
- **`Tag` drops `title`.** Explanations go in visible text or a `Toggletip`.
- **Dismissable `Tabs` hard-code a Close icon**, which clashes with Archive for "put away".
- **`TextArea` does not auto-grow.**
- **Fields are 14 px.** Northstar raises them to 16 px on coarse pointers, so iOS does not
  zoom.
- **The `Header` is `position: fixed`** and ignores safe areas.
- **`HeaderGlobalBar` grows with its content** and pushes actions off a 390 px header.
- **`Tabs` are capped at one 64 px row.** Wrapped tabs are unreachable on a phone.
- **List-box menus are as narrow as their trigger.** Northstar widens them.
- **Batch-action bars overflow a phone.**
- **Chart legends filter on click by default.** `@carbon/charts-react` calls `setData` whenever
  the data array's identity changes, so chart data is memoised by content.
- **Preview-only exports:** `IconIndicator`, `ShapeIndicator`, `Card` and `PageHeader`.

## Where each part goes

**Overview**

- **Scope bar.**
  - Breadcrumb crumbs, each opening a `ScopePicker`: a Popover with Search and a
    ContainedList, using the listbox pattern rather than today's `role=menu` with an input
    inside.
  - `ContentSwitcher` for the time range, and a ghost Close button to clear the scope.
- **Search.** `Search` at 16 px on touch, and a `ContentSwitcher` for the five kinds that never
  collapses into a dropdown. Hits are list items with a kind `Tag`, with matches marked from
  split text, never from HTML.
- **Quota.**
  - `UsageMeter` per window, and a status `Tag` whose colour and words come from the same
    field.
  - History is a `Sparkline`, or a `LineChart` if the strip may grow.
  - The `BurnDown` keeps the forecast's honesty rules: hours of work, red only for a crossing
    that survives a quiet week, the basis and band stated, expired windows never showing old
    numbers.
- **All accounts and headline.** Northstar's `StatTile` and `StatGrid`, with each tile's
  meaning as visible text. The Errors tile gets a visible "List the N errors" button.
- **Activity.** `StackedBarChart` in the model-family colours, with a fixed legend order and a
  legend that does not filter.
- **What the model read, context window, threads.** `ShareBar` with its legend.
- **Models, account split, projects, sessions, tools.** `BarList`, never a DataTable.
- **Requests.**
  - `RequestRow` cards.
  - `ContentSwitcher` and `SelectableTag` for the filters.
  - "Show more" as a tertiary button, not Pagination.
  - The p95 threshold as visible helper text.

**Settings**

- **Accounts.**
  - One `Tile` per account with Use, Edit and Remove always visible, never in an overflow
    menu.
  - Editing happens in the card, with the server's own token-prefix rules as `invalidText`.
  - Remove is confirmed in a danger `Modal`.
- **Add account.**
  - A visible "Add account" button opens the form in place.
  - Each account type is a `RadioTile` carrying its own explanation, so `claude setup-token`
    is explained where the choice is made.
  - The sign-in URL and the profile command sit in a `CodeSnippet`.
- **Setup.** One `Toggle` each for start at login, route every project and manage the wrapper,
  with the explanation and the outcome (`InlineLoading`, `InlineNotification`) beside each.
- **Network.** A `Toggle` bound to the server's state, not flipped optimistically. The LAN
  address goes in a `CodeSnippet`, with "set a password first" and the `AGENT_ROUTER_BIND`
  caveat next to the switch.
- **Password.** Two `PasswordInput`s, and a Remove button that is always visible and asks for
  confirmation.
- **Notifications.**
  - Apple fields, and the key in a `TextArea` that is never pre-filled.
  - Registered phones as compact rows.
  - Test results that survive the next refresh.
  - The API already reports Firebase, which the page ignores today.
- **First-boot password.** A small `ComposedModal`, opened under the same conditions as today.

**Sessions**

- **List.** One `ContainedList` per project, headed by its dot, count and age. Rows are real
  buttons with a visible Archive (or Restore) action, and the folds keep their open state
  across refreshes.
- **Tabs.** A tab strip over the feed, each tab carrying its status dot (see Decision I for
  its close glyph).
- **Feed.**
  - Custom bubbles and rails, and Markdown as React elements: Carbon lists, `Link` (https
    only), and `CodeSnippet` for fences.
  - A `Clamp` that clamps before it measures.
  - Terminal blocks as multi-line `CodeSnippet`s.
- **Question form.**
  - A `Tile` with `RadioButtonGroup` or `CheckboxGroup`. Group names come from `useId()`,
    which fixes the shared `q0` group.
  - An "Other" field.
  - Submit guarded against a double tap.
- **Permission card.** A `Callout` with the command in a wrapping `CodeSnippet` and every
  choice visible as a button: Allow, each of the router's suggestions, Deny.
- **Switchers.**
  - Model, effort and mode as inline `Select`s, which give a phone its native picker.
  - "Other…" and Bypass open Carbon `Modal`s. Native `prompt()` and `confirm()` are blocked
    in VS Code's sandboxed fallback, where those two do nothing today.
- **Composer.**
  - A `TextArea` that grows to 120 px, at 16 px on touch.
  - Send, a visible Stop in `--ar-warn` while a turn is stoppable, and the side question.
- **Find.** `Search` with a This session / Everywhere switch, hits as clickable tiles, and its
  state in the URL so Back closes it.
- **Notices.** `InlineNotification` next to whatever they concern, on a timer of our own.

**Responsive rules** for all three: desktop-first and the same product on a phone. No
horizontal scroll at 360 px, so transcripts, tool output and paths wrap anywhere. No fact
reachable only by hover. Fields at 16 px and targets at 48 px on coarse pointers (Northstar's
rules). The live view in two panes at 1,056 px and wider (Decision H).

## Defects found on the way

Settled where the port meets them, unless marked *now* or *server*.

| Defect | Where | Kind | Fix |
| --- | --- | --- | --- |
| `esc()` escapes `& < >` but not quotes, and its output lands in attributes: a Markdown link's `href` and an AskUserQuestion option's `value`. Model-written text can add event handlers to a page that can approve prompts and send messages. | `live.html:464`, `739`, `827` | security | **fixed**, `29c5eb3`; React by construction |
| `/live` can be framed by any web page | `live.rs:1204-1214` | security | frame policy |
| Refusals answered as HTTP 200 + `{error}` are shown as "set", "saved" or "done"; a pasted `.p8` can be lost silently | `index.html:810-815`, `2257`, `2375`, `2063-2069` | data loss | fetch layer; **server** to answer non-2xx |
| A scope change mid-refresh lets the older reply paint last | dashboard refresh loop | wrong figure | scope-keyed queries |
| A slow read for one session draws its model, effort and mode under another, and the next switch goes to whichever session is on screen | `live.html` facts and switchers | broken control | results tagged by session |
| The "older" and "archived" folds close on every refresh | `live.html:631`, `698`, `709` | broken control | state in React |
| Every question form in the feed shares one radio group | `live.html:826` | broken control | `useId()` |
| "Not yet attributed" subtracts a session list capped at 200 | `api.rs:606` | wrong figure | **server** count |
| `/api/accounts` ignores `probe=1` and checks every credential on every call | `api.rs:1324` | cost | **server** |
| The spend tooltip's full split (`04efe6b`) is overwritten by an older call | `index.html:1465`, `1475` | wrong figure | port |
| A "tight" verdict without a central estimate prints "runs out ~20000d before reset" | `index.html:1224-1227` | wrong figure | port; iOS already guards it |
| Missing data shown as 0 % and unpriced models as $0 | `index.html:1127-1128`; `pricing.rs:122-124` | wrong figure | "—"; **server** for spend |
| The quota tag's colour and its words can come from different fields | `index.html:1154-1156` | wrong figure | port |
| Remove password, Remove key and Turn off network act on one click, so a LAN viewer can lock itself out | dashboard Settings | ux | danger `Modal` |
| Errors written into hidden elements (a failed Restore, "no longer listed"); Find left open beside the list after Back | `live.html` | ux | port |

## The order of work

0. **Independent of the rest.** `esc()` is fixed (`29c5eb3`). The two existing check scripts
   go into CI (AGEN-0016), so the contracts stay guarded while the rewrite runs.
1. **Contracts.** A `docs/web-console-contracts.md` in the shape of Northstar's `docs/13`, one
   section per area, regressions marked. The pure TypeScript modules, with Vitest specs ported
   from the 45 checks and their fixtures. Shared Markdown and grouping fixtures the phones can
   also test against.
2. **Scaffold and serving.**
   - `ui/` copied from Northstar's setup, with the Carbon partials this app renders, the fonts,
     g10 and g100 with the `--ar-*` tokens re-validated, and the shell.
   - On the Rust side: `rust-embed`, the routes and headers, and the `build.rs` policy.
   - `build-target.sh` and CI.
   - The new app is mounted at `/next/` (`/next/`, `/next/live`, `/next/settings`), so the old
     pages keep serving until the switch.
3. **The custom components and the concepts registry**, each with a spec and a harness page
   at phone and desk widths.
4. **Overview.** 5. **Settings.** 6. **Sessions.** Once 1–3 land, these three can proceed in
   parallel, in separate worktrees.
7. **Parity and switch.**
   - Every contract green, screenshots at 360 and 390 px and at 1,440 px in both themes, the
     palette re-validated, and an accessibility pass.
   - `/` and `/live` switch to the new app, and the old pages, the check scripts and the two
     `include_str!` go.
   - README and ARCHITECTURE are updated, and the phones' parity tests still pass.

**Size.** Northstar's rewrite replaced about 8,000 lines of components and specs. This one
replaces 4,244 dense lines and should land in the same range. Every phase is reviewable on its
own.

**While both exist**, the old pages stay authoritative and keep taking fixes. Any behaviour
change made there during the migration goes into the contracts list in the same commit; the
hosting work's server-mode notice on the live view is the first.

**The hosting work** (`serve --server`, a Host allow-list, machine-local features off) has
landed (`e621f67` to `27d41ba`). The console reads what the server can do, from `/api/auth`'s
`mode`, rather than assuming a Mac. In server mode it hides the login item, the wrapper, the
network bind and the Keychain-backed account sources, as the old pages now do (`8b277fc`).
Served on a public name, the frame policy and the CSP matter more.

## Tracked in Northstar

The work is filed in Northstar's `agent-router` project as the epic **AGEN-0001**. It has
twelve slices, two server defects the console exposes, and one item that runs the old
pages' checks in CI until they go. Every clause is a command that
fails today and passes when its slice is done. The screen slices are held to
`docs/web-console-contracts.md`: every contract id in a screen's section must be named in a
spec or a shot script.

| Item | Slice | Waits on |
| --- | --- | --- |
| AGEN-0002 | The contracts, written down area by area | — |
| AGEN-0003 | `ui/` on Northstar's tooling, theme and shell, with the `--ar-*` tokens | — |
| AGEN-0004 | The two check scripts ported to Vitest over React-free modules | 0003 |
| AGEN-0005 | Shared Markdown and grouping fixtures for web, iOS and Android | 0004 |
| AGEN-0006 | The custom components and the concepts registry | 0003 |
| AGEN-0007 | The ⌘K palette over the router's search | 0003 |
| AGEN-0008 | The embedded console served at `/next/` behind the guard | 0003 |
| AGEN-0009 | CI and `build-target.sh` build the console | 0003 |
| AGEN-0010 | Overview | 0002, 0004, 0006 |
| AGEN-0011 | Settings | 0002, 0003 |
| AGEN-0012 | Sessions | 0002, 0004, 0006 |
| AGEN-0013 | The switch: `/` and `/live` serve the console, and the old pages go | 0007–0012 |
| AGEN-0014 | Defect: refusals answered as HTTP 200 | — |
| AGEN-0015 | Defect: the unattributed count, and `probe=1` ignored | — |
| AGEN-0016 | The two existing check scripts run in CI until the switch retires them | — |

## What we take from Northstar

Paths are in Northstar's `ui/`, as of its commit `d46713f`.

**Copy as it stands:**

- **Tooling.**
  - `eslint.config.mjs`, with its restricted-import rule pointed at the charts.
  - `src/test-setup.ts`. It installs an in-memory Storage because Node 22+'s own localStorage
    hides jsdom's, and stubs ResizeObserver, matchMedia and scrolling.
  - `src/test-utils.tsx`: a fresh QueryClient per test, the Carbon theme, a MemoryRouter, mock
    responses for ok, refused, unreachable and empty, and `stubNarrow` and `stubCoarse`.
  - `scripts/notices.mjs`, which writes the third-party notices the MIT release needs. Drop its
    entries for Northstar's graph libraries.
- **App plumbing.**
  - `app/query.ts` together with `data/held.ts`, whose `useRead` keeps the last data through a
    failed refetch and reports value, age, stale and unreachable. Also `lib/freshness.ts`.
  - `app/tick.ts`, so ages keep moving.
  - `app/notify.ts` with `notices.tsx`: toasts only for actions whose control has gone, and
    refusals stay beside their control.
  - `app/ErrorScreen.tsx`, reworded.
  - `app/theme.ts`.
  - `lib/viewport.ts`: `useNarrow` at Carbon's own 41.98 rem, and `useCoarse`.
- **Components.** `StatGrid`, `LiveBadge`, `EmptyState`, `PageHeader` (Carbon's is still a
  preview), `Page` and its sections, `Card`, and `FormModal`. Fix FormModal's phone rule on
  the way: it uses `body-01`, which compiles to 14 px where its comment promises 16 px, and
  `ReasonModal` has the same mistake.
- **The shell itself.** `app/Shell.tsx` and `Shell.module.scss`, including its CSS fixes: a
  full-width header, a bounded global bar (Carbon's `HeaderGlobalBar` otherwise grows and
  pushes actions off a 390 px header), the phone margin and 48 px links. Its sign-in and
  log-out actions are left out, because the router's LAN gate is HTTP Basic.
- **The palette.** `screens/Palette/`: 406 lines, a 637-line spec, the ⌘K shortcut and the hit
  grouping. Its hits come from agent-router's search instead of Northstar's items.
- **Settings.** `screens/Settings/Layout.tsx`, the URL-driven tabs, with agent-router's
  sections.

**Adapt:**

- **`App.tsx`.** Keep its lazy screens and an error element per route, so a crashed screen
  leaves the shell alive. Leave out its `/login` route: the router's LAN gate is HTTP Basic,
  which the browser handles.
- **`app/live.ts`.** Its update-mode discovery and its fall back to polling suit the Overview.
  The transcript needs an appending stream with resume, which Northstar never built.
- **`StatTile`** for the quota and headline tiles. Its null is an em dash and "not a zero".
- **`Population`**: shown of total, exclusions and truncation, worded for agent-router's lists.
- **The details `Drawer`**, a drawer on a desk and a bottom sheet on a phone, as a tool-call
  inspector.
- **`StatusChart`**, the one chart. It reads colours from CSS variables and the theme from the
  `<html>` class, falls back to a table if it throws, and has a "Show as table" toggle. Give
  the charts their own lazy chunk; Northstar's sit inside its Dashboard chunk.
- **The fixture harness.** It replaces `fetch` with fixtures and records any read it could not
  answer. Add an `EventSource` fake for the live view.
- **`scripts/shoot-workmap.mjs`.** It drives Chrome over the DevTools protocol and shoots
  1440×900 and 390×844@2x in both themes. Add 360 px, where today's overflow is worst.
- **`scripts/check-chunks.mjs`.** It checks placement and has no byte budget; give it one.

**Skip** `Markdown.tsx`, which has no links or tables. Keep its rule: a React tree, never
HTML.

**Northstar's contracts that also bind here** (its `docs/13-console-contracts.md`):

- **Missing is not zero.** A null is never shown as zero. Data kept from a failed refresh shows
  its age, and every display says when it was measured.
- **Refusals stay at the control.** A refusal appears in the server's words beside the control
  it came from, and server rules are checked before sending with the server's own numbers.
- **Refreshing.** One clock drives every re-read, a tab re-reads when it becomes visible, the
  polling timer runs until the stream opens, and every SSE event name is registered.
- **Both widths.** Every fact and action exists at both widths. Hover-only affordances are
  visible where there is no hover, targets are 48 px and fields 16 px.
- **The URL.** Filters live in the URL, and typing replaces the history entry rather than
  adding one.
- **Links.** A link to one entry lands on it.

**Northstar's bugs not to repeat** (hashes in Northstar's history):

- **`EventSource`.**
  - `1374e53`: an error while it is still CONNECTING means the browser is reconnecting.
    Checking only for CLOSED showed "live" over dead data.
  - `bcd16e2`: only registered event names dispatch. The live view already uses a named
    `status` event.
- **`940a783`, the one that matters most here.** Changing a route parameter keeps the screen
  mounted, so drafts were saved onto the next project. Key the Sessions screen on the session
  id, or a composer draft or a pending answer goes to the wrong session.
- **Stale values.**
  - `28efc66`: a dialog that captured its row when it opened re-sent stale data.
  - `7fb9192`: a page index clamped only in its handler read "101–40 of 40" after a poll.
- **`d834fd6`, tests.** TanStack Query notifies on its own timer, so a test must wait for the
  condition, not one tick.
- **`4b65e8b`, long text.** One unbroken URL widened a 390 px page to 1,119 px. Transcripts,
  tool output and paths need `overflow-wrap: anywhere`.
- **Tabs and touch targets.**
  - `29472eb`: Carbon's tabs are capped at one 64 px row, so wrapped tabs cannot be reached on
    a phone. That matters for the session strip.
  - `dbf9481`: targets were under 48 px in wrapped tabs, with a hard-coded `size="sm"`,
    ComboBox icons and chart legends. Size by pointer type, not by width.
- **`16d666d`, the breakpoint.** A 671 px media query in script disagreed with Carbon's
  41.98 rem at fractional zoom. Use `viewport.ts`.
- **`7b6e4ab`, page length.** A capped 200-row list made a page 30,803 px tall. The request
  feed holds 300.

**Size.** Northstar's `dist` is 2.9 MB in 80 files: 2.7 MB of assets and 252 kB of fonts. Its
entry script is 572 kB (180 kB gzipped), its main stylesheet 704 kB (71 kB gzipped), and the
charts' stylesheet 241 kB, loaded with its dashboard. Agent-router's should be similar, and it
is what the binary grows by unless the embed is compressed.

## Decisions

Decided on 2026-10-05:

| | Question | Decision |
| --- | --- | --- |
| A | The look | **Northstar's shell and theme**: its topbar, ⌘K search and palette, rail and phone drawer, white and g100 themes, and Settings pages. Agent-router's colour coding is kept as tokens on top. |
| B | How the bundle is built | **In CI and `build-target.sh`**, embedded with `rust-embed`, and a release build without it fails. Not a committed `ui/dist`, whose hashed files churn history and collide between sessions. Not a `build.rs` running pnpm, which puts Node on every cargo call. |
| C | How the work starts | **Filed in Northstar** as a scoped, structured set of items before any of it is built. |
| — | The `esc()` injection | **Fixed** in today's live view, commit `29c5eb3`. A jsdom run of the page's own `md()` showed a Markdown link gaining an `onmouseover` before the fix, and none after. `check-live-ui.js` still passes. It goes live with the next release; it was not deployed on its own. |
| D | Frame policy | **Only VS Code's own schemes may frame the console** (`frame-ancestors vscode-webview: vscode-file:`), and `X-Frame-Options` is dropped. Checked in VS Code before it ships. |
| E | Settings | **Pages under `/settings`**, in Northstar's layout |
| F | Markdown | **Keep the web's semantics**, the phones' reference, pinned by shared fixtures (AGEN-0005). Converging the three clients is a separate decision. |
| G | Defects | **Fixed where the port meets them**; server-side fixes as separate small commits (AGEN-0014, AGEN-0015) |
| H | Sessions on a desk | **Two panes from 1,056 px**, the list beside the feed |
| I | Folds and put-away | **Archive is a visible action** on every row and tab, with the Archive icon, and **the archived shelf is visible**, not folded |
| J | Rollout | **Behind `/next/` until parity**, then `/` and `/live` switch |

The hosting work landed on 2026-10-05, in `e621f67` to `27d41ba`: `serve --server`, `build.rs`,
the image, the chart and the CI stages. The serving and CI slices build on it.

## Unverified

- Everything the readers measured or ran and nobody re-checked: the phone overflow widths, the
  contrast ratios, the defects they confirmed in jsdom, the header's 118 px height. The `esc()`
  injection, the missing frame headers on `/live`, the "set" after a refused password and the
  documents' promises were re-read against the code for this note.
- That `X-Frame-Options: DENY` breaks Simple Browser on VS Code before 1.109. This follows from
  the headers and was not run.
- That Carbon and Carbon Charts inject no `<style>` elements, and that Vite emits no inline
  script, which a strict CSP needs. Both are checked on the first build.
- That `frame-ancestors vscode-webview: vscode-file:` admits Simple Browser and its fallback.
- Whether a macOS CI runner is registered, and whether pnpm is reachable in the Linux job
  without installing it.
- The crates `rust-embed` adds, and how much the binary and VSIX grow. Northstar's sizes come
  from its `dist` as built on 2026-09-24, before its graph engine was added.

## Sources

- Northstar, at commit `d46713f`: `docs/adr/0015-carbon-react-is-the-console.md`,
  `docs/13-console-contracts.md`, `ui/` (package.json, vite.config.ts, src/styles.scss,
  src/fonts.scss, src/app/, src/components/, src/lib/viewport.ts, scripts/) and the `fix(ui)`
  commits named above.
- Carbon: [Carbon Design System](https://carbondesignsystem.com),
  [`@carbon/react`](https://github.com/carbon-design-system/carbon/tree/main/packages/react),
  [IBM Plex in Carbon](https://github.com/carbon-design-system/carbon/blob/main/docs/guides/ibm-plex.md),
  [Carbon fonts](https://github.com/carbon-design-system/carbon/blob/main/packages/styles/scss/fonts/README.md);
  component behaviour read in `@carbon/react` 1.116 and `@carbon/charts-react` 1.27 as
  installed under Northstar's `ui/node_modules`.
- [rust-embed](https://docs.rs/crate/rust-embed/latest).
- This repository: `assets/index.html`, `assets/live.html`, `src/main.rs`, `src/api.rs`,
  `src/live.rs`, `src/guard.rs`, `src/auth.rs`, `scripts/check-*.js`, `scripts/build-target.sh`,
  `scripts/package-vsix.sh`, `.gitlab-ci.yml`, `vscode/src/`, `menubar/src/main.rs`, the phone
  tests named above, [session liveness](session-liveness.md) and the
  [product assessment](product-assessment-2026-09-13.md).
