# Agent Router for VS Code

**What this extension does on your machine, in one paragraph.** It installs the Agent
Router binary into `~/.agent-router/bin` and runs it as a background process that keeps
running after every editor window is closed, listening on `127.0.0.1:8787` (or on every
interface, if you turn that on, for the phone app). With your say-so, it registers the
router as a login item, sets `ANTHROPIC_BASE_URL` in Claude Code's settings file so Claude
Code's API traffic goes through the router, and sets Claude Code's documented
`claudeCode.claudeProcessWrapper` setting so Claude is launched through the router's
wrapper. The router signs requests with your own Claude credential; nothing leaves your
machine except the requests Claude Code was already making.

Agent Router records what Claude Code sends and receives, forecasts when the 5-hour and
7-day limits will be hit, switches between accounts, follows live sessions from a phone,
and pushes a notification when a session waits on you.

## Commands

- **Agent Router: Open Dashboard** and **Open Live Sessions** — the router's own pages,
  in an editor tab.
- **Agent Router: Start or Update the Router** — runs the bundled build if none is
  serving, or hands over to it without dropping a request if an older one is.
- **Agent Router: Set Up on This Machine** — login item, routing, and the process wrapper.
- **Agent Router: Remove from This Machine** — undoes exactly that. Your data stays.

## Settings

`agentRouter.port`, `agentRouter.listenOnNetwork`, `agentRouter.manageService`,
`agentRouter.configureClaudeCode`, `agentRouter.showStatusBar`.

Removing the extension does not stop or remove the router. Run **Remove from This
Machine** first if you want that.
