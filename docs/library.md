# The Library: skills and other agent components, the same on every machine

2026-10-07. A plan for a store of reusable agent components on a hosted router. It starts
with Claude Code skills, slash commands and subagents. The machine agent keeps every
connected machine in step with it. Things get in four ways: an upload from the console, an
import from a GitLab or GitHub repository, a forge's push webhook, or an agent publishing
an improved version. The router can tell a machine to update at once through the
connection the machine's agent already holds open, so no machine opens a port.

Written for the maintainer, and as the contract the slices are built against. The work is
Northstar's epic AGEN-0042 ([Tracked in Northstar](#tracked-in-northstar)). Nothing in this
document is built yet.

## Owner's decisions (2026-10-07)

| Question | Answer |
| --- | --- |
| What reaches machines when a version arrives on its own (a forge push, a webhook, an agent publishing) | **Trust per source.** Each forge source and each publish token is set to "goes live" or "needs approval". An upload from the console always goes live. A version waiting for approval shows in the console with a diff. |
| What happens to a Library item edited on a machine | **The Library wins.** The next sync puts the Library's copy back. To keep a change, publish it first. The agent keeps the last copy it replaced in its data directory, so nothing an agent wrote is lost without trace. |
| Which machines get which items | **Every item goes to every machine**, except a machine that opted out of the Library altogether. Collections can come later. |
| What is in the first cut | Skills, slash commands and subagents. Upload, forge import, webhooks, agent publishing, push and pull, the console. **Usage seen by the proxy.** |
| The first item | **Agent Router's own skill.** It tells an agent on any machine what the router and the Library are and how to publish an improvement. Later it is how agents on different machines find each other and talk. |

## The shape

```
console ─┐                                   ┌── forge (GitLab, GitHub): import, poll
phone   ─┼─ https ─▶  hosted router  ◀───────┤
CI      ─┘ (token)    the Library           └── forge push webhook  /hooks/library/<source>
                          ▲      │
            https: manifest,     │ wss, outbound from the machine:
            blobs, report,       │ "sync now" (POST /agent/library/sync)
            publish              ▼
                      machine agent ──writes──▶ ~/.claude/skills, commands, agents
                      (each machine)            (Claude Code reads them; skills live)
```

- **The router owns the Library.** Items, their versions, where each came from, which is
  live, and what each machine last reported.
- **The agent pulls.** It fetches the manifest and the blobs over HTTPS with the password
  it already holds. It checks each digest and swaps each item into place whole.
- **The router can push without a port.** It sends one tunneled request, "sync now", down
  the WebSocket the agent opened ([the tunnel](hosting-machine-agent.md#the-tunnel)). The
  request carries no content. The agent answers by pulling, as it would at any other time.
- **The Library is a hosted router's feature**, like Machines. A local router does not
  offer it, and its routes and screen say why.

## An item

**Kinds and where they go:**

| Kind | A version is | Placed at |
| --- | --- | --- |
| `skill` | a directory with `SKILL.md` and any supporting files | `~/.claude/skills/<name>/` |
| `command` | one Markdown file | `~/.claude/commands/<name>.md` |
| `subagent` | one Markdown file with its frontmatter | `~/.claude/agents/<name>.md` |

On Windows the same layout is under `%USERPROFILE%\.claude`. A name is lowercase letters,
digits and hyphens, at most 64 characters. A skill directory needs no prefix: it sits beside
the user's own skills.

**Claude Code reads skills live.** A changed or new skill under `~/.claude/skills` reaches a
session that is already running. So a push changes what every open session can use, with no
restart. Commands are now the same mechanism as skills. Whether a new subagent needs a new
session is unverified.

**A bundle is checked before it is stored, and refused in words otherwise:**
- A skill has a `SKILL.md` whose frontmatter has a `name` equal to the item's and a
  `description`. Commands and subagents have their own frontmatter.
- No symlinks, no absolute paths and no `..`. At most 1 MiB per file and 8 MiB per bundle.
- Packing a directory leaves out a machine's own clutter wherever it appears: `.git`, `.venv`,
  `venv`, `__pycache__`, `node_modules` and `.DS_Store`. The vault skill on this Mac carries a
  whole Python virtual environment, which would otherwise be refused for its size.
- Nothing that looks like a credential. That means a private key block, or an Anthropic,
  GitLab or GitHub token. The refusal names the file and the line. Credentials belong in the
  vault, never in something copied to every machine.

**A version is immutable and content-addressed.** The files are packed in a canonical form:
sorted, with fixed times and modes, keeping only the executable bit. The sha256 of that
form is the version's digest. The same files make the same digest wherever they came from,
so publishing identical content makes no new version. Blobs live in the main store, keyed
by digest. The two-file store is unchanged.

**Each version records:**
- **Its number** within the item, from 1.
- **Its state:** `live`, `pending`, `rejected` or `superseded`. At most one version of an
  item is live, and machines get only live versions.
- **Where it came from**, one of:
  - an upload from the console;
  - a forge, as repository, commit and path;
  - a publish token, by name;
  - a machine, with the Claude Code session that published it when there was one;
  - the router's own build.

  A version published by a session links to that session in the console. The router
  already knows the session's transcript.

**An item has one source of truth.** An item imported from a repository takes new versions
only from that repository. An upload or a publish to it is refused, and the refusal names
the repository, because the change belongs there. The router's own skill takes versions only
from the router's build. Every other item takes uploads and publishes.

## Ways in

### Upload

The console takes a chosen folder (packed in the browser into a tar), a `.zip`, a `.tar.gz` or
a single `.md`, and the API takes the same body (a `.tar.gz`, a `.zip`, a `.tar` or one Markdown file).
- **It goes live at once.**
- **A wrapper is dropped.** A folder around everything, and macOS's `__MACOSX` and `._` files,
  are taken off.
- **The newest content is made live, not stored twice.** Content that is already the item's
  newest version makes that version live.
- **A rejected version never goes live.** Uploading its content again makes it a new version.

### Forges and sources

**A forge** is a base URL with a kind, GitLab or GitHub, and an optional token. The model is
Northstar's (`crates/northstar-forge`, Settings → Forges), copied with its source commit
noted:
- The token is kept in the store and never returned by any read.
- A check says ok, unauthorised, insufficient scope or unreachable.
- GitLab needs `read_api`.
- GitHub needs no token for a public repository, which is how `anthropics/skills` would
  come in.

**A source** is a forge, a repository, a ref (a branch or a tag), a path within the
repository, and a trust setting ("goes live" or "needs approval").

**Importing:**
- **No git.** The image is distroless and has no `git`, so an import fetches the ref's
  archive through the forge's API:
  - GitLab: `GET /projects/:id/repository/archive.tar.gz?sha=<commit>&path=<path>`
  - GitHub: `GET /repos/:owner/:repo/tarball/<commit>`, filtered to the path
- **What counts as an item** under the path:
  - every directory holding a `SKILL.md` is a skill;
  - `commands/*.md` are commands;
  - `agents/*.md` are subagents;
  - a path that is itself a skill directory is that one skill.
- **What changes.** Each item found becomes a version only if its digest changed. An item
  that left the repository is shown on the source, not removed by itself.
- **Polling.** The poller asks each source for its ref's head every five minutes and
  imports only when the head moved. "Import now" does the same at once.

**Rules that keep a token where it belongs:**
- **https only.** A forge's base URL must be https; plain http is accepted on loopback alone.
- **A moved forge loses its token.** Moving a forge's address drops its token unless a new one
  is sent with it.
- **GitHub's archive host never sees it.** GitHub's archives come from another host, which is
  never shown the token.
- **The router names itself.** Every request carries an `agent-router/<version>` user agent,
  which GitHub asks for.

**Size.** GitHub's tarball is always the whole repository: at most 64 MiB as it arrives and
256 MiB unpacked, of which only the source's path is kept. A source in a bigger repository
belongs on GitLab, which is asked for the path alone, or in a smaller repository.

**Owning an item:**
- **Taking over an upload.** An item uploaded or published here can be taken over by an
  import only once it is archived.
- **Removing a source.** Its items become anyone's to change.
- **Items that left the repository** are listed as `gone` in the import's result and stay
  where they are.

Northstar's own skill would come in this way, from its repository's `plugin/skills/northstar`.

### Forge webhooks

`POST /hooks/library/<source>` re-imports that source.
- **Each source has its own secret.** It is generated, shown once, and can be rotated.
- **The forge proves itself.** GitLab with `X-Gitlab-Token`, GitHub with
  `X-Hub-Signature-256` (an HMAC of the body). Both are compared in constant time.
- **Only a push to the tracked ref imports.** A ping is answered. A delivery id already seen
  imports nothing.
- **The answer is quick.** The route answers 202 and imports in the background.
- **This is the one new open path.** Besides `/_router/health` and the installer, it is the
  only path a hosted router answers without the password, because a forge cannot hold the
  password.
- **Polling stays** as the reconciler, for a webhook that never arrived.

**Setting a hook up.** `POST /api/library/sources/{key}/secret` makes a new secret, which
stops the old one, and answers it once with the hook's URL. Give both to the forge: GitLab's
"Secret token", or GitHub's "Secret" with content type `application/json`.

**What a hook is answered:**
- `202` when it imports;
- `200` for a ping, a push to another ref, an event that is not a push, or a delivery seen
  already;
- `401` when it does not prove itself;
- `403` when the source has no secret yet;
- `404` for an unknown source;
- `413` for a body over 1 MiB.

Delivery ids are kept for a week, in the store, so a restart does not forget them.

### Agents publishing

- **From a connected machine:** `agent-router library publish <dir>`.
  - It reads the server and password from `agent.json`, so no new secret goes on the machine
    and nothing is asked for.
  - It checks the bundle the way the router will before sending it.
  - It sends the machine id, and `CLAUDE_CODE_SESSION_ID` when the command runs inside a
    Claude Code session (Claude Code sets it for Bash tool commands).
  - Publishes from machines need approval unless the owner sets them to go live. That is one
    setting for all machines.
- **From anywhere else**, such as CI or a cloud session: **a publish token.**
  - A token is shown once and kept only as a hash.
  - It is scoped to kinds or to named items, and carries its own trust setting.
  - It can publish and nothing else. Every other route, reading included, refuses it.
  - Revoking it stops it at once.

  Today the only credential is the router password, and handing that to a pipeline would
  hand over everything.

**How it is built:**
- **What the command sends.** `agent-router library publish <dir | file.md> [--kind …]
  [--name …]` names the item:
  - a directory is a skill named after the directory;
  - a file is a command or a subagent, by the `commands/` or `agents/` directory it sits in.

  It sends the canonical tar with `X-Agent-Router-Machine`, `X-Agent-Router-Label` and, from
  a Claude Code session, `X-Claude-Code-Session`.
- **Tokens.** A token is `arl_` and 40 hex characters. The guard looks a bearer with that
  prefix up only on `POST /api/library/items/{kind}/{name}/versions`. Anywhere else it is a
  wrong password, and the guard answers 401.
- **Scope.** Named items, when any are given; otherwise kinds, when any are given; otherwise
  anything. A publish outside the scope is a 403 naming both.
- **Revoking** keeps the token's row, so what it published still names it. Its last use is
  shown.
- **Machine publishes** are one setting, `GET/PUT /api/library/settings`
  `{machine_publishes: "approval" | "live"}`. It defaults to `approval`.

### The router's own skill

`skills/agent-router/` in this repository is built into the binary. At start a hosted router
publishes it as the item `skill/agent-router`:
- live;
- with its build as the provenance;
- with no new version when the content is unchanged.

The skill describes commands of the agent from the same build, and a machine's agent comes
from the router's image, so they always agree. A router rolled back to an older build
publishes the older skill, which is still correct for it.

The skill covers:
- what the router and the machine agent are;
- what the Library is, and that the Library wins;
- how to publish an improved skill;
- that an item from a repository is changed in the repository;
- that credentials go in the vault.

It keeps only portable frontmatter, as Northstar's skill does. It is meant to grow a section
on finding and messaging agents on other machines when that feature exists.

## Reaching machines

### The sync

The agent asks for `GET /api/library/manifest`. That is the live versions, as
`{kind, name, version, digest, size}`, with an ETag, so an unchanged Library costs one 304.
For each item:
1. **Its place matches the digest:** nothing to do. A copy of the same content that the
   Library did not place is taken as the Library's from then on.
2. **Otherwise:**
   - It downloads `GET /api/library/blobs/<digest>`, checks the digest, and unpacks into
     `~/.claude/.agent-router-staging`. That is on the same filesystem as `skills/`, and
     outside it, so Claude Code never lists a half-written skill.
   - What is at the target goes:
     - The agent's own unchanged copy of an older version simply goes.
     - Anything else is moved aside to `<data-dir>/library/replaced/<kind>/<name>`, keeping
       the last one only, and named in the report. That covers a copy edited on the machine,
       a skill of that name the Library did not place, and a link. The Library wins.
   - It renames the staged copy into place, whole. Windows retries a rename that a reader
     briefly blocks.
3. **A symlink at the target** is moved aside as a link. It is never written through. The
   Northstar skill on this Mac is such a link, into a repository.
4. **An item no longer in the manifest** is removed, but only where the ledger
   (`<data-dir>/library/ledger.json`) says the agent placed it. A copy edited since is kept
   aside, as a replaced one is.

One sync runs at a time per data directory (`library/sync.lock`). The last report is kept as
`<data-dir>/library/last-sync.json`.

**The agent syncs:**
- when it connects;
- every 15 minutes after its last sync;
- when told to (AGEN-0046);
- from `agent-router library sync`, which prints what it did.

**Reports.** After a sync it ran on its own (at a connection, or on its timer), the agent
posts its report to `POST /api/library/report`. A sync the router asked for answers with the
report instead.

**`agent-router library status`** prints one line per item: name, kind, version, and where it
stands here. That is one of:
- `in step`;
- `behind`;
- `changed here`;
- `not the Library's`;
- `failed`, with why;
- `withdrawn`.

**Opting out.** `agent install --no-library` writes `"library": false` into `agent.json`, and
that machine's agent never writes into `~/.claude` nor asks the router for the Library.
`agent install --library` takes it back. `agent status` says which.

### Push

- **The tunnel's route list grows by exactly one**, `POST /agent/library/sync`. It runs a
  sync and answers with its report. The agent's other routes stay as they are: no setup,
  no shim, no files.
- **`POST /api/library/sync`** tells every connected agent, or the one named, and answers
  with each machine's report.
- **Making a version live** does the same for every connected agent, with nobody pressing
  anything.
- **A machine whose agent predates the Library** answers that route 404. It is named as such,
  with "re-run the installer" (MA-C4's advice).

**The trust this adds, stated plainly.** Until now the tunnel's contract was that the router
"can never ask a machine for more than its sessions". With the Library, the router can also
cause files to be written into every machine's `~/.claude`. Those files are instructions an
agent follows, and scripts a skill may run.

What bounds it:
- the push carries no content;
- the agent writes only the three places above, never through a link;
- every blob's digest is checked;
- automatic sources can be held for approval;
- a machine can opt out.

A router whose store is taken over could still serve a bad skill. Signing versions with a key
the router never holds is listed under [Later](#later).

### Each machine's state

The router keeps each machine's last report in the store. Machines themselves are in memory
only today, so without this an offline machine's state would be lost at every restart.

`/api/machines` gains `library` for each machine: `{state, behind, failed, replaced, at_ms}`.
The `state` is one of:
- `in_step`;
- `behind`, with how many live items its last sync did not leave at their live version;
- `failed`;
- `replaced`, when its last sync put items back over local copies (they are named);
- `predates`;
- `opted_out`;
- `not_yet`, for a machine that has said nothing of the Library.

**Offline machines.** One that was offline when a version went live reads as behind until it
reconnects and syncs. Asked to sync now, it is answered as offline.

**After a restart.** A router started again reads each machine's last word from the store, so
a machine back online shows its state at once.

## Usage

The router sees every request and response, so it can say how the Library is used. Nothing
on the machines can say that.

**Counted against a Library item of that name, by day, machine and project:**
- a `Skill` call in a response;
- a subagent launched by name (`subagent_type`);
- a command or skill a person typed (`<command-name>`).

Each is counted once, not again from every later request that repeats it in history. The
count comes from where the search index first places a block: one spot in a session, which
the requests repeating it do not place again (`library::usage`, `db::write_blocks`). So a use
is taken from the request that first carries the call, a second or two after the response
that made it. Counting as requests are recorded and rebuilding from the index find the same
uses.

A typed command counts only when it opens the turn, as Claude Code sends one. A summary or a
file that mentions the tag further in is not a use. A typed name counts for the skill of that
name when the Library holds one, and for the command otherwise. A skill the model calls is
not also counted as typed: Claude Code follows a `Skill` call with the skill's text, and
with no command tag.

Every name an item could have is kept, so a name counts before its item joins. Whether it
counts against an item is decided when usage is read. A name that is not a Library item is
not counted as one.

**Machines.** A session's machine is the one that listed it, which the store keeps in
`sessions.machine`: every machine's agent reports the sessions it lists every minute
(`src/places.rs`), so a use stays placed after a restart. A use's project is the session's
project, its repository else its directory (`sessions.project`). A session no machine has
listed counts under "a machine not known".

**The listing's cost.** Every turn carries the list of skills with their descriptions.
The router measures each Library item's entry and the Library's total, as an estimate of
tokens added to every turn. A long description shows as a cost, not just as text.

The listing is a user text block: a `<system-reminder>`, the head "The following skills are
available for use with the Skill tool:", a blank line, then a `- name: description` line per
skill. A description may run on to further lines. The router measures the newest such
block it has carried. `src/library/fixtures/skill-listing.txt` pins the form. It was copied
from a request Claude Code sent on 2026-10-07, with only Claude Code's own skills kept.
Commands and subagents are not in that listing, so it costs them nothing.

**History from before the Library.** Usage is rebuilt from the bodies the router already
stores: their blocks, in the search index. A skill uploaded today shows the weeks it was
already used. A hosted router runs the rebuild once at start, after the router before it has
let go of the store. It resumes from `settings`' `library.uses_after`.

**In the console:**
- each item's use over time;
- items not used in 30 days are marked;
- "the Library adds about N tokens to every turn" heads the screen.

## The console

**Library** (`/library`) is in the rail after Machines, on a hosted router only.
- **The list.** Each item shows:
  - its kind, name, description and live version;
  - its source;
  - its pending versions;
  - its machines in step and behind;
  - its use.
- **An item's page:**
  - `SKILL.md` rendered with the console's Markdown, and the files;
  - versions with a diff between any two;
  - each version's provenance, linking to the repository commit or to the session that
    published it;
  - approve and reject, make live (which is also a rollback), archive, and Sync now.
- **Upload.** Choose a folder, a zip, a tar.gz or a Markdown file. A folder is packed in the
  browser, leaving a machine's clutter out as the router does.
- **Contracts.** They are a new **Library** section of
  [web-console-contracts.md](web-console-contracts.md), `LB-C1` onwards.

**Machines** gains each machine's Library state and a Sync now per machine.

**Settings** gains two pages:
- **Forges** (`/settings/forges`), Northstar's page copied.
- **Library** (`/settings/library`):
  - sources, with trust, the last import, Import now, and the webhook URL with its secret
    (shown once, rotatable);
  - publish tokens: create (shown once) and revoke;
  - whether publishes from machines go live.

The phones are not in this epic. iOS's Machines screen can show the Library state later.

## API

All routes need the password unless marked otherwise.
- **Refusals.** Write routes refuse in the router's usual words, `{"ok":false,"error":…}`.
- **Upload size.** An upload body may be up to 8 MiB, and one that declares more is refused
  before it is read. The ingress allows 32 MiB.
- **Files.** A version's files are served as plain text under `Content-Security-Policy:
  sandbox`, so a skill's HTML never runs as a page of the router.
- **Local mode.** A router on a developer's machine answers every `/api/library` path with a
  409 saying the Library is a hosted router's.

| Route | What it does |
| --- | --- |
| `GET /api/library` | Every item: live version, pending count, machines in step and behind, use |
| `GET /api/library/items/{kind}/{name}` | The item, its versions with provenance and state, and the live version's file list |
| `GET /api/library/items/{kind}/{name}/versions/{n}/files/{path}` | One file of one version |
| `POST /api/library/items/{kind}/{name}/versions` | Publish a version (tar.gz, zip or one Markdown file). Password or a publish token |
| `POST …/versions/{n}/live`, `…/versions/{n}/reject` | Approve or roll back, and reject |
| `POST /api/library/items/{kind}/{name}/archive` | Take the item off every machine |
| `GET /api/library/manifest`, `GET /api/library/blobs/{digest}` | What agents sync from |
| `POST /api/library/report` | An agent's report of what it holds |
| `POST /api/library/sync` | Sync now: every connected machine, or `{machine}` |
| `GET/PUT/DELETE /api/library/forges[/{key}]`, `POST …/{key}/check` | Forges |
| `GET/PUT/DELETE /api/library/sources[/{key}]`, `POST …/{key}/import`, `POST …/{key}/secret` | Sources, Import now, and a new webhook secret |
| `GET/POST/DELETE /api/library/tokens[/{id}]` | Publish tokens |
| `GET/PUT /api/library/settings` | Whether publishes from machines go live |
| `GET /api/library/usage` | Use per item by day, machine and project, and the listing's cost |
| `POST /hooks/library/{source}` | **Open**: the forge's webhook, proved by the source's secret |

Agent side, through the tunnel only: `POST /agent/library/sync`.

## Later

These are not filed. Each is its own decision.
- **Collections:** machines subscribing to groups of items, for bench-only skills.
- **More kinds:**
  - output styles;
  - MCP servers and hooks, which change `settings.json` and so run code at Claude Code's
    start;
  - whole Claude Code plugins. The agent could serve them as a local-directory marketplace,
    which Claude Code loads at session start.
- **Signed versions:** a key the router never holds, so a taken-over router cannot place a
  skill.
- **Approvals on the phone:** a push notification when a version waits.
- ~~The agent updating itself~~: no longer later. It is filed on its own as **AGEN-0055**:
  the agent compares its binary with the one its router hands out, swaps it in by rename,
  and restarts as the new one. The router can also ask it to check, through the tunnel.
- **Agents on different machines finding and messaging each other**, introduced to agents
  by the router's own skill.
- **Project-scoped items**, placed into a repository's `.claude/`. These usually belong in
  the repository itself.

## Tracked in Northstar

The work is the epic **AGEN-0042** in Northstar's `agent-router` project, with twelve slices
linked by `blocks` edges. Every clause is a command that fails today and passes when its
slice is done:
- **Rust slices** name the tests that define them, and require exactly that many to pass.
- **Console slices** require every contract id in their section to be named in a spec.
- **AGEN-0054** ends on an observation of this Mac, so the epic closes attested.

| Item | Slice | Waits on |
| --- | --- | --- |
| AGEN-0043 | The store: kinds, bundle checks, the credential scan, content-addressed immutable versions, provenance | — |
| AGEN-0044 | The API: read, upload, approve, make live, roll back, archive, the manifest and blobs; local mode says why | 0043 |
| AGEN-0045 | The agent's sync: placement, digests, swap in whole, the Library wins, links, the ledger, opt-out, `library status` | 0044 |
| AGEN-0046 | Push over the tunnel (`/agent/library/sync`), sync now, push on going live, each machine's state kept | 0045 |
| AGEN-0047 | Forges (GitLab, GitHub) and sources: archives without git, discovery, trust, polling, one source of truth | 0043, 0044 |
| AGEN-0048 | Forge webhooks: the open route, GitLab's token and GitHub's signature, replay, refs | 0047 |
| AGEN-0049 | Publishing: `agent-router library publish` with the session, and write-only publish tokens | 0044 |
| AGEN-0050 | Agent Router's own skill, built in and published from the build | 0044, 0049 |
| AGEN-0051 | The console's Library screen, and the Library state on Machines | 0044, 0046 |
| AGEN-0052 | Settings: Forges, and Library (sources, webhooks, tokens, machine publishes) | 0047, 0048, 0049 |
| AGEN-0053 | Usage seen by the proxy, the listing's cost, history from stored bodies, and its console half | 0043, 0051 |
| AGEN-0054 | End to end, the docs, and this Mac holding the router's skill in step (observation) | 0046, 0048, 0050–0053 |

**What can run in parallel.** After AGEN-0044 there are three lines, each in its own
worktree:
- the machine line: 0045, then 0046;
- the forge line: 0047, then 0048;
- publishing and the skill: 0049, then 0050.

## Unverified

- Whether Claude Code picks up a new subagent in `~/.claude/agents` in a running session.
  Skills are documented as live; commands are now skills.
- Whether github.com can reach the hosted router's public URL, for GitHub webhooks. GitLab
  on the same network can.
- Whether a GitLab token with `read_repository` alone can download a path-filtered archive.
  Northstar requires `read_api`, and this plan follows it.
- That the agent on Windows places items correctly. Like the rest of the Windows agent, it
  will have been built and not run.

## Sources

- Claude Code skills: locations, live reload, frontmatter, precedence —
  https://code.claude.com/docs/en/skills.md
- Plugin loading and local-directory marketplaces —
  https://code.claude.com/docs/en/plugins/loading.md,
  https://code.claude.com/docs/en/plugins/host-marketplace.md
- `CLAUDE_CODE_SESSION_ID` — https://code.claude.com/docs/en/env-vars.md
- GitLab repository archive API — https://docs.gitlab.com/api/repositories/#get-file-archive
- GitLab webhooks and `X-Gitlab-Token` — https://docs.gitlab.com/user/project/integrations/webhooks/
- GitHub tarball API — https://docs.github.com/en/rest/repos/contents#download-a-repository-archive-tar
- GitHub webhook signatures — https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries
- The tunnel and the agent — [hosting-machine-agent.md](hosting-machine-agent.md);
  `src/agent.rs` `routes()`, `src/tunnel.rs`, `src/machines.rs`
- Northstar's forge model — Northstar `crates/northstar-forge`, `crates/northstar-server/src/settings.rs`
