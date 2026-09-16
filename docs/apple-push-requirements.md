# What Apple push notifications need

Finding, 2026-09-13, from setting up push for the Agent Router iPhone app. Everything
that can be built in is built in; what remains is issued by Apple to the account
holder's login only.

| Piece | Who can make it | Built in? |
| --- | --- | --- |
| APNs signing key (`.p8`) | only the account holder, in the developer portal (Keys → + → Apple Push Notifications service → Register → Download); handed out exactly once | imported by the router by itself from `~/.agent-router/AuthKey_<key id>.p8`: key id from the file name, team id from the app's first registration |
| Push-enabled App ID for `ca.whnet.agent-router` | only the account holder: an Apple ID signed into Xcode lets automatic signing register it, or the portal by hand | no; one-time |
| Provisioning profile carrying `aps-environment` | same; the team's wildcard profile cannot carry it | no; one-time, then Xcode keeps it |
| Router that talks to Apple | us | yes (HTTP/2, ES256 provider token, sandbox vs production per device) |
| App with the entitlement and registration | us | yes; the app reports its team and bundle id when it registers |

Facts established along the way:

- The team is a paid Apple Developer Program membership: 365-day
  profiles and App Store profiles exist. A free personal team cannot use push at all.
- Token-based auth (one `.p8` key) serves every app on the team, sandbox and
  production, and does not expire; the older per-app push certificate does none of
  that and is not supported by the router.
- No API creates the key; the App Store Connect API can register App IDs and profiles
  but itself needs a key created in the portal, so a portal login is unavoidable once.
- Signing into Xcode with the Apple ID, then a signing build for a generic iOS
  destination with provisioning updates allowed, creates the App ID with the Push
  capability and the profile without the phone being connected.
- The phone must be unlocked for any device build or install ("Developer Disk Image
  is not mounted" otherwise).
- A tap on a notification must be handled with the completion-handler delegate forms,
  completed on the main queue: the `async` forms have their completion invoked from a
  background executor and UIKit's state-restoration work inside it asserts main-thread
  and aborts, which looks like "tapping does not open the app". Verified by a simulator
  UI test that taps a local notification carrying the same payload.
