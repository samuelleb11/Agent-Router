# What Google push notifications need

Finding, 2026-09-14, from building the Android app. The companion to
`apple-push-requirements.md`, and a much shorter story: everything is free, self-serve, and
re-issuable.

| Piece | Who can make it | Built in? |
| --- | --- | --- |
| Firebase project | anyone with a Google account, free Spark plan, no card | no; one-time |
| `google-services.json` (the app's client config) | same, from the console | no; one-time, dropped into `android/app/` |
| Service-account key (the router's sender credential) | same, from Project settings → Service accounts | imported by the router by itself from any `*.json` in `~/.agent-router/` whose `"type"` is `"service_account"` |
| Router that talks to Google | us | yes (`src/fcm.rs`: RS256 assertion → OAuth token → FCM HTTP v1) |
| App that registers and draws its own notifications | us | yes |

## The one-time setup

1. console.firebase.google.com → **Add project**. The Spark plan is enough; messaging is
   free at any volume this will ever see.
2. **Add an Android app**, package name `ca.whnet.agentrouter` (no hyphen — Android package
   segments cannot contain one, which is why it differs from the iOS bundle id
   `ca.whnet.agent-router`). Download `google-services.json` into `android/app/`.
3. Project settings → **Service accounts** → **Generate new private key**. Drop the JSON
   into `~/.agent-router/` on the Mac. The router takes it in by itself — the project id is
   read out of the file — makes the file `0600`, and says so in the dashboard and under
   Settings → Notifications on the phone. Pasting it into the settings screen also works.

Then Settings → Notifications on the phone, and "Send test".

## Facts established along the way

- **Unlike Apple's `.p8`, a service-account key can be re-issued at will.** Losing one costs
  a click, not an escrow. It is still kept out of the tree by `.gitignore` and out of the
  transcript by the drop-in import.
- **Google wants the assertion exchanged, where Apple takes it directly.** An RS256 JWT
  signed with the service account's key buys an access token from `oauth2.googleapis.com`,
  and *that* is the bearer on the send. It lasts an hour and is reused for fifty-five
  minutes. This is the only structural difference from the APNs path.
- **`ring` signs it with no new dependency**, and needs no `openssl` fallback: Google's
  `private_key` is a complete PKCS#8 RSA key, where Apple's `.p8` omits the public half.
- **Messages are data-only in both directions.** A message carrying a `notification` block
  is drawn by the system whenever the app is backgrounded, and never reaches the code that
  knows how to tag it, group it by session, or take it down again. So the router sends the
  parts and the app builds every banner itself.
- **A token's shape is not proof of anything.** An FCM registration token is opaque — any
  long enough run of URL-safe characters passes the validator. Google's `UNREGISTERED` is
  what actually settles it, and the device is forgotten then. The validator exists to keep
  a stray paste out of the table, not to authenticate a phone.
- **Only two refusals mean "forget this device":** `UNREGISTERED` (404) and
  `SENDER_ID_MISMATCH` (403), plus the one `INVALID_ARGUMENT` whose complaint names
  `message.token`. A bare 400 is deliberately *not* enough — it is equally what a malformed
  payload earns, and treating that as a dead device would empty the whole register the first
  time we shipped a bad message.
- **`google-services.json` is not a secret** — it ships inside the APK — but it names one
  person's Firebase project, so it is supplied per machine rather than committed.

## What will still stop a notification arriving

Not Google's doing, and worth knowing before blaming the code:

- **Samsung and several other makers** put apps they decide are unused to sleep, which stops
  background delivery outright. The app offers the battery-optimisation exemption under
  Settings and says why. This is the single most likely cause of "it just stopped working"
  on the Note 9.
- **A force-stopped app** receives nothing until it is launched again. Swiping it away from
  the recents list is *not* force-stop; "Force stop" in Settings is.

Both are survivable and neither is worked around: the app asks `/api/push/pending` what is
still waiting every time it comes to the front, and clears up whatever the push missed.
