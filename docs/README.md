# Findings and research

Research, decisions and findings about Agent Router that are not descriptions of the
code. One document per finding, dated in its first lines, sources with URLs at the end,
and an "unverified" list where the research could not confirm something. Written for
the maintainer who has to act on them. What the code does and why is in
[README.md](../README.md) and [ARCHITECTURE.md](../ARCHITECTURE.md).

| Document | Date | One line |
| --- | --- | --- |
| [Multiple providers and generic clients](multi-provider-clients.md) | 2026-10-11 | Three-provider routing and registration are implemented; Claude Code is the primary CLI/native VS Code client, with catalogue and full session-control acceptance still open. |
| [Codex setup and protocol translation](codex-and-protocol-translation.md) | 2026-10-11 | Claude Code and Codex route by model across Messages and Responses, with explicit limits for provider-specific features. |
| [Shipping Agent Router as a VS Code extension](vscode-extension-research.md) | 2026-09-13 | It is possible; the extension supervises a machine-wide daemon rather than being one, and Claude Code's own process-wrapper setting replaces the file-level shim. |
| [What publishing a VS Code extension costs](vscode-extension-publishing-costs.md) | 2026-09-14 | Nothing in fees; accounts, a build Mac, and time. |
| [What Apple push notifications need](apple-push-requirements.md) | 2026-09-13 | One signing key made once by the account holder, a push-enabled App ID from a signed-in Xcode, nothing else provisioned after install. |
| [Product assessment: better and uniform](product-assessment-2026-09-13.md) | 2026-09-13 | Nine things, ordered: commit the work, one source of truth for how the router runs, one web app, identical vocabulary across surfaces, then product bets. |
| [Where the subscription quota goes, and what would stretch it](quota-token-economics.md) | 2026-09-14 | Ordinary turns are 87% of the 5-hour meter and a turn at 700K context costs 2.4 times one at 150K; caching is already at 99%, so the levers are compaction, effort below `max`, fewer turns, and no web search on Fable. |
| [Prompt caching and Claude Code settings, as they affect the router](prompt-caching-and-claude-code-settings.md) | 2026-09-14 | What Claude Code puts on the wire, the caching rules that decide what a turn costs, and the settings that control compaction, effort, MCP scope and headless forks. |
| [Session liveness](session-liveness.md) | 2026-09-29 | The transcript cannot tell generating, background work, a permission prompt or a closed process from idle; Claude Code's own session registry can, so the router reads it first and derives every session's state in one place. |
| [Hosting Agent Router on Kubernetes](hosting-on-kubernetes.md) | 2026-10-05 | A pod can proxy, record, analyse and push on its own; the live view and steering need a component on each developer machine that dials out. Phase 1 is `serve --server`, an image, a chart and a pipeline in Northstar's shape. |
| [The machine agent: live sessions through a hosted router](hosting-machine-agent.md) | 2026-10-05 | Each developer machine runs the same binary as an agent that dials out over one WebSocket, so a hosted router can list, follow and steer every machine's sessions, on macOS and Windows. Tried end to end on macOS; the Windows build has not run on Windows yet. |
| [Moving the web UI to React and IBM Carbon](web-console-carbon.md) | 2026-10-05 | Feasible, and still one binary: Carbon's theme with our colour coding layered on as tokens, a dozen custom visualisations, and the bundle embedded with rust-embed. The 500-odd behavioural contracts and the phones' parity tests come first, and today's live view has a script injection to fix now. |
| [The Library: skills and other agent components, the same on every machine](library.md) | 2026-10-07 | A plan, not built: a hosted router keeps skills, commands and subagents fed from uploads, GitLab and GitHub, webhooks and agents; each machine's agent pulls them into `~/.claude`, and the router says "sync now" down the agent's own outbound tunnel. Filed as AGEN-0042. |

The build and packaging chain is in the README, section "Building and packaging for
Mac and Linux", and in ARCHITECTURE.md, "The editor extension".
