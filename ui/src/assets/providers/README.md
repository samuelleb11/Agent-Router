# Provider marks

Each provider's own mark, shown beside its name wherever the console, the VS Code status bar
and the phones name a provider (AGEN-0124, AGEN-0125). Each mark is used only to say which
provider an account, a request, a model or a session belongs to. The files are copied as
published, and not modified.

| File | Mark | Owner | Taken from |
| --- | --- | --- | --- |
| `anthropic.svg` | Anthropic's logomark, one path, no fill colour of its own: it takes the text colour, black on light and white on dark | Anthropic, PBC | Simple Icons 15.0.0, `icons/anthropic.svg` (https://cdn.jsdelivr.net/npm/simple-icons@15.0.0/icons/anthropic.svg); Anthropic's own press kit is https://www.anthropic.com/press-kit |
| `kimi-code.svg` | Kimi's "K only" mark for a light background: a black K with Kimi's blue dot | Moonshot AI | Kimi's brand guide, https://moonshotai.github.io/Branding-Guide/scenarios/04-k-only/k-only-light.svg |
| `kimi-code-dark.svg` | The same mark for a dark background: a white K with the blue dot | Moonshot AI | https://moonshotai.github.io/Branding-Guide/scenarios/04-k-only/k-only-dark.svg |

The VS Code extension ships the same files in `vscode/media/providers/`, for its status bar's
tooltip (AGEN-0138), and one more: `anthropic-dark.svg`, the same path filled white for a dark
editor theme, as the console draws Anthropic's mark in the text colour.

The iPhone app ships the same three files, unmodified, in
`ios/AgentRouter/Sources/Assets.xcassets/` (AGEN-0143): `ProviderAnthropic` renders as a
template in the text colour, and `ProviderKimi` holds the light file with the dark one as its
dark appearance.

SHA-256, as copied on 2026-10-10:
- `anthropic.svg`: `1cc599f6ebce2016dc388cf84e54a52c6b13487655c7e243554d654c7bce1882`
- `kimi-code.svg`: `937616746780d0c7c4bda92820db39f503f6a3e9b7cfebe438a5fb61b62d3991`
- `kimi-code-dark.svg`: `3821ae593eebce6940992f56e2fee3f55a08bbbba78713303c23ea714d44c5f9`

## What the owners' terms say (read 2026-10-10)

- **Anthropic's Trademark Guidelines** (https://www.anthropic.com/legal/trademark-guidelines):
  - "You may only use our trademarks as specifically permitted by us and only in materials we approve beforehand."
  - The marks may not be altered, and may not imply Anthropic's sponsorship, endorsement or affiliation.
  - Permission is asked of marketing@anthropic.com by those with an existing business relationship.
- **Kimi's logo usage terms** (https://www.kimi.ai/policies/logo-usage-terms, linked from the
  Kimi Brand Book at https://www.kimi.ai/resources/kimi-brand):
  - "Assets are licensed solely for editorial, media, and non-commercial promotional uses."
  - The logo may not be modified, and may not imply endorsement, sponsorship or affiliation without prior written consent.
  - Moonshot AI may revoke permission at any time.

**Sam's decision (2026-10-10).** Agent Router is his personal tool. Showing each provider's own
mark to identify it is common practice and within the law. He uses the official marks and
answers for it himself.

Agent Router's `main` mirrors to GitHub under MIT, and these files go with it. The MIT licence
covers Agent Router's own code. It grants nothing in these marks, which remain their owners'.
