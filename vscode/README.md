# Agent Router for VS Code

Agent Router records what Claude Code sends and receives, forecasts when the 5-hour and
7-day limits will be hit, switches between accounts, follows live sessions from a phone,
and pushes a notification when a session waits on you. This extension runs it in one of
two modes: on your machine (local mode, the default) or on your server (remote mode).

## Local mode: the router on your machine

**What this extension does on your machine, in one paragraph.** It installs the Agent
Router binary into `~/.agent-router/bin` and runs it as a background process that keeps
running after every editor window is closed, listening on `127.0.0.1:8787` (or on every
interface, if you turn that on, for the phone app). With your say-so, it registers the
router as a login item, sets `ANTHROPIC_BASE_URL` in Claude Code's settings file so Claude
Code's API traffic goes through the router, and sets Claude Code's documented
`claudeCode.claudeProcessWrapper` setting so Claude is launched through the router's
wrapper. The router signs requests with your own Claude credential; nothing leaves your
machine except the requests Claude Code was already making.

**Where your data lives: on your machine**, in `~/.agent-router`. That is everything the
router records (the requests Claude Code makes and the responses it gets, which hold your
prompts and code), your accounts, the router's password and its logs.

## Remote mode: the router on your server

Set `agentRouter.server` to the address of a hosted Agent Router, most easily with
**Agent Router: Connect to a Server…**. It asks for the address and the router's
password, checks both with the server, and keeps the password in VS Code's secret
storage, never in settings. Use an `https://` address: over plain `http://` the password
crosses the network in the clear.

A machine set up from the server's install script, which runs `agent-router agent
install`, needs none of that: the first window to open finds the server and the password
in the agent's own file, keeps the password in secret storage, sets `agentRouter.server`
and the wrapper, and says so, instead of starting a router on this machine.

No router runs on this machine then. Instead:

- Claude Code's API traffic goes to the server, which signs it with an account added
  there and forwards it to Anthropic. The one-time set-up (`agent-router agent install`)
  writes `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` (the password) into Claude
  Code's settings file.
- This machine runs only the agent, which connects out to the server and listens on
  nothing. It lets the phone and the server's live page follow and steer the Claude Code
  sessions on this machine. The set-up registers it as a login item unless you decline;
  every window also starts it (`agent-router agent --ensure`).
- Claude is still launched through the wrapper, which is how the agent types into a
  session.
- The status bar shows the server's quota, and the dashboard and the live page open in
  your browser.

**Where your data lives: on your server.** It keeps everything the router records: the
requests Claude Code makes and the responses it gets (your prompts and code), your
accounts and their tokens, the password, and the phones registered for notifications.
On this machine are the binaries and the agent's own file in `~/.agent-router` (the
server's address, the password and a machine id, readable by you alone), the password in
VS Code's secret storage and in Claude Code's settings file, and Claude Code's
transcripts, which stay where Claude Code writes them. The agent tells the server which
sessions run here, and relays a session's transcript through the server when someone
follows it from the phone or the live page.

On Windows only remote mode is available. There `~/.agent-router` is
`%USERPROFILE%\.agent-router`, and the programs are `agent-router.exe` and
`claude-wrapper.exe`.

## Commands

- **Agent Router: Open Dashboard** and **Open Live Sessions**: the router's own pages, in
  an editor tab in local mode and in your browser in remote mode.
- **Agent Router: Connect to a Server…**: switch this machine to remote mode, or give it
  a changed password.
- **Agent Router: Disconnect from the Server**: undo the set-up (`agent-router agent
  uninstall`), forget the password on this machine, and go back to local mode. The
  server and what it keeps are not touched.
- **Agent Router: Start or Update the Router** (local mode): runs the bundled build if
  none is serving, or hands over to it without dropping a request if an older one is.
- **Agent Router: Set Up on This Machine**: login item, routing, and the process
  wrapper, for the router on this machine or for the server.
- **Agent Router: Remove from This Machine**: undoes exactly that. Your data stays.

## Settings

`agentRouter.server` (empty for local mode), `agentRouter.configureClaudeCode`,
`agentRouter.showStatusBar`, and, in local mode only, `agentRouter.port`,
`agentRouter.listenOnNetwork` and `agentRouter.manageService`.

Removing the extension does not stop or remove the router or the agent. Run **Remove
from This Machine** first if you want that.
