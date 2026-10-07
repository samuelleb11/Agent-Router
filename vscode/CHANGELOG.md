# Changelog

## Unreleased

- Remote mode: `agentRouter.server` and **Connect to a Server…** point this machine at a
  hosted router. The password is kept in VS Code's secret storage; the extension installs
  and starts the agent (`agent-router agent`) instead of a router, reads the quota from
  the server, and opens its pages in the browser. **Disconnect from the Server** undoes it.
- A machine set up by the server's install script (`agent-router agent install`) is taken
  over when a window opens, before any local router starts: the password is copied from
  the agent's file into secret storage, `agentRouter.server` is set, and Claude Code is
  handed the wrapper.
- Windows: `.exe` programs, the agent started without a console window, and a
  `win32-x64` package.

## 0.1.0

- First packaging of the router as an extension: bundled binaries per platform, a
  status bar quota readout, dashboard and live pages in an editor tab, one-time set-up
  and removal commands.
