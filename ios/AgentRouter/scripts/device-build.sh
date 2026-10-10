#!/bin/sh
# Build, install and launch the app on the paired iPhone.
#
# DEVICE and DEVELOPMENT_TEAM must be set; neither is kept in the repo.
#   xcrun devicectl list devices     # to find a UDID
#
#   scripts/device-build.sh          # with push (needs an Apple ID signed into Xcode)
#   PUSH=0 scripts/device-build.sh   # without the push entitlement, for a Mac that has none
#   PROFILE="name" scripts/device-build.sh
#                                    # with push, signed by a development profile made by
#                                    # hand in the portal for ca.whnet.agent-router (its
#                                    # name as shown there), for a Mac with no Apple ID
#                                    # in Xcode; double-click the .mobileprovision first
#
# xcodegen records the push capability on the project as well as in the entitlements
# file, so a build without it needs a project generated without it — not a build
# setting override, which Xcode's signing checks ignore.
set -e
cd "$(dirname "$0")/.."
# No apostrophe in that message: inside ${VAR:?word} the shell parses the word, and a
# lone ' opens a quote it never closes — which breaks the whole script, not the message.
DEV="${DEVICE:?set DEVICE to the UDID of the target iPhone (xcrun devicectl list devices)}"
: "${DEVELOPMENT_TEAM:?set DEVELOPMENT_TEAM to your Apple Developer team id}"
if [ "${PUSH:-1}" = "0" ]; then
  python3 - <<'PY'
import re
s = open("project.yml").read()
s = re.sub(r"\n    entitlements:\n      path: Sources/AgentRouter.entitlements\n      properties:\n        aps-environment: development\n", "\n", s)
open("project.nopush.yml", "w").write(s)
PY
  xcodegen generate --spec project.nopush.yml --quiet
  rm -f project.nopush.yml Sources/AgentRouter.entitlements
else
  xcodegen generate --quiet
fi
SIGNING=""
if [ -n "${PROFILE:-}" ]; then
  SIGNING="CODE_SIGN_STYLE=Manual PROVISIONING_PROFILE_SPECIFIER=$PROFILE"
fi
# shellcheck disable=SC2086
xcodebuild -project AgentRouter.xcodeproj -scheme AgentRouter -configuration Debug \
  -destination "platform=iOS,id=$DEV" -derivedDataPath build -allowProvisioningUpdates $SIGNING build 2>&1 \
  | grep -E "error:|BUILD (SUCCEEDED|FAILED)" | head -5
# devicectl prints "Failed to load provisioning paramter list … error" on every run, even
# a plain `list devices`. Dropping that one line is what lets the install receipt — or a
# real error — be the line you actually see.
xcrun devicectl device install app --device "$DEV" build/Build/Products/Debug-iphoneos/AgentRouter.app 2>&1 \
  | grep -E "installationURL|error" | grep -v "No provider was found" | head -1
xcrun devicectl device process launch --terminate-existing --device "$DEV" ca.whnet.agent-router 2>&1 | tail -1
# Leave the tree on the canonical project.
xcodegen generate --quiet
