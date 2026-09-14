# What publishing a VS Code extension costs

Finding, 2026-09-14. No direct fee anywhere in the chain. The costs are accounts,
infrastructure and time, and most of them are already carried.

## Publishing itself is free

- **Visual Studio Marketplace:** no fee to create a publisher or to publish. A Microsoft
  account and an Azure DevOps organization used only for sign-in; the organization's
  first five users are on the free Basic tier, so a solo publisher pays nothing. The
  publisher id is permanent once created. Personal access tokens retire on 1 December
  2026; afterwards the pipeline signs in with an Entra ID identity (`vsce publish
  --azure-credential`). A setup change, not a cost.
- **Verified publisher badge:** free, gated by time. The extension must have been
  listed for six months and the domain registered for six months, proven by a DNS TXT
  record; review within five business days. example.com qualifies on age already.
- **Open VSX:** free. An Eclipse Foundation account linked to GitHub, a namespace
  matching the publisher id, and a license file in the package.

## Where money could enter, and why it does not have to

- **Apple signing and notarization:** not required. VS Code's installer sets no
  quarantine on extracted files, so the bundled binary runs as it is. A Developer ID
  signature, if ever wanted for polish, comes with the Apple Developer Program already
  paid for the iOS app.
- **Build machines:** the GitLab pipeline needs a Mac for the two Mac packages.
  Registering an owned Mac as a shell runner is free; rented hosted macOS runners are
  the paid alternative and are not needed. The Linux job runs in a standard container.
- **A public repository is not required.** The manifest's repository link should point
  somewhere reachable or be omitted; the packaging script passes
  `--allow-missing-repository` when there is no remote.

## The real cost

Time: the account setup, the disclosure paragraph at the top of the listing, and the
six-month clock on the verified badge.

## Sources

- https://code.visualstudio.com/api/working-with-extensions/publishing-extension
- https://azure.microsoft.com/en-us/pricing/details/devops/azure-devops-services/
- https://github.com/eclipse/openvsx/wiki/Publishing-Extensions
- https://dev.to/diana_tang/complete-guide-publishing-vs-code-extensions-to-both-marketplaces-4d58
- https://ice-ice-bear.github.io/posts/2026-02-27-vsce-extension-deploy/
