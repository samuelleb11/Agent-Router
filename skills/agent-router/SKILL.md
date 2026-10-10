---
name: agent-router
description: Work on a machine connected to an Agent Router — what the router and this machine's agent are, how the router's Library places skills, commands and subagents in ~/.claude (and puts back edits made here), and how to publish an improved or new one so every machine gets it. Use when writing or improving a skill, command or subagent that other machines should have, when a skill in ~/.claude/skills changed back under you, or when asked about Agent Router, its agent, or its Library.
license: MIT
---

# Agent Router, its agent, and its Library

Agent Router is a proxy between Claude Code and Anthropic. Its owner runs one, hosted, and
every machine they work on connects to it. Through it the owner follows and steers sessions
from a phone or a browser, and keeps the **Library**: the skills, slash commands and
subagents every machine shares.

On this machine, the router's **agent** (`agent-router agent`) does that work. It keeps one
outbound connection to the router and listens on nothing. To see it:

```
agent-router agent status
```

That prints the server, this machine's id and name, whether the agent runs, and whether this
machine takes the Library.

## The Library on this machine

The agent keeps these in step with the router's Library:

- `~/.claude/skills/<name>/`
- `~/.claude/commands/<name>.md`
- `~/.claude/agents/<name>.md`

It syncs when it connects, every 15 minutes, and whenever the router says a version went
live. Skills reach sessions that are already running.

```
agent-router library status   # each item, and where it stands here
agent-router library sync     # sync now, and print what changed
```

**The Library wins.** An item the Library placed and you edited in place is put back at the
next sync. The copy it replaced is kept in `~/.agent-router/library/replaced/`, the last one
only. So never leave an improvement only in `~/.claude`: publish it.

## Publishing an improvement, or a new item

```
agent-router library publish ~/.claude/skills/<name>                  # a skill's directory
agent-router library publish ./my-command.md --kind command           # a command
agent-router library publish ~/.claude/agents/<name>.md               # a subagent, by where it sits
```

- It reads the router's address and password from the agent's own settings, and asks for
  nothing.
- It checks the item here first, as the router will.
- It signs the version with this machine and with this Claude Code session. The owner sees
  which session wrote it, and can read that session.
- It says whether the version went **live**, and so reaches every machine now, or **waits
  for approval** in the console. Publishes from machines wait unless the owner trusts them.
  Until then, the next sync here puts back the live version. Your change is safe in the
  waiting version, and in `~/.agent-router/library/replaced/`.

**Items that come from a repository.** If the publish is refused with "comes from
<repository> (<ref>)", that item is imported from a repository. Change it there, with a
commit, and the router imports it within minutes. This skill itself "comes with the router's
build": it lives in Agent Router's own repository, at `skills/agent-router/`.

## What makes an item publishable

- **A skill** is a directory with a `SKILL.md` whose frontmatter has `name` (equal to the
  directory's name) and `description`.
  - Claude Code decides when to use a skill from its description, and every turn of every
    session carries every description. Say when to use it, in as few words as do that.
- **A command** is one Markdown file with a `description` in its frontmatter.
- **A subagent** is one Markdown file with `name` and `description` in its frontmatter.
- **Names** are lowercase letters and digits, with single hyphens between them.
- **Size.** At most 1 MiB a file and 8 MiB in all.
- **No links.** A link is refused, not followed.
- **Left out when publishing a directory:** `.git`, `.venv`, `venv`, `__pycache__`,
  `node_modules` and `.DS_Store`.

**Never put a credential in an item.** The Library copies every item to every machine and
keeps every version for good. A publish carrying something shaped like a private key, or an
Anthropic, GitLab or GitHub token, is refused, naming the file and line. Credentials belong
in the vault (use the `vault` skill if this machine has it). Have the item's scripts read
them at the point of use.

## Not yet

The router cannot yet find agents on other machines, nor pass messages between them. Do not
look for a command that does. When it can, this skill will say how.
