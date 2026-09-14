# Findings and research

Research, decisions and findings about Agent Router that are not descriptions of the
code. One document per finding, dated in its first lines, sources with URLs at the end,
and an "unverified" list where the research could not confirm something. Written for
the maintainer who has to act on them. What the code does and why is in
[README.md](../README.md) and [ARCHITECTURE.md](../ARCHITECTURE.md).

| Document | Date | One line |
| --- | --- | --- |
| [Shipping Agent Router as a VS Code extension](vscode-extension-research.md) | 2026-09-13 | It is possible; the extension supervises a machine-wide daemon rather than being one, and Claude Code's own process-wrapper setting replaces the file-level shim. |
| [What publishing a VS Code extension costs](vscode-extension-publishing-costs.md) | 2026-09-14 | Nothing in fees; accounts, a build Mac, and time. |
| [What Apple push notifications need](apple-push-requirements.md) | 2026-09-13 | One signing key made once by the account holder, a push-enabled App ID from a signed-in Xcode, nothing else provisioned after install. |
| [Product assessment: better and uniform](product-assessment-2026-09-13.md) | 2026-09-13 | Nine things, ordered: commit the work, one source of truth for how the router runs, one web app, identical vocabulary across surfaces, then product bets. |
| [Where the subscription quota goes, and what would stretch it](quota-token-economics.md) | 2026-09-14 | Ordinary turns are 87% of the 5-hour meter and a turn at 700K context costs 2.4 times one at 150K; caching is already at 99%, so the levers are compaction, effort below `max`, fewer turns, and no web search on Fable. |
| [Prompt caching and Claude Code settings, as they affect the router](prompt-caching-and-claude-code-settings.md) | 2026-09-14 | What Claude Code puts on the wire, the caching rules that decide what a turn costs, and the settings that control compaction, effort, MCP scope and headless forks. |

The build and packaging chain is in the README, section "Building and packaging for
Mac and Linux", and in ARCHITECTURE.md, "The editor extension".
