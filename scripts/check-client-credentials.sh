#!/usr/bin/env bash
# Credential compatibility contracts for web, phone and VS Code clients. Requires the
# native development toolchains; model requests and real credentials are never used.
set -uo pipefail
cd "$(dirname "$0")/.."
result=0
pnpm --dir ui exec vitest run src/screens/Settings/Router.spec.tsx src/screens/Settings/FirstBoot.spec.tsx src/screens/Settings/Password.spec.tsx src/screens/Settings/Network.spec.tsx || result=1
npm --prefix vscode test || result=1
if [[ -z "${JAVA_HOME:-}" && -d '/Applications/Android Studio.app/Contents/jbr/Contents/Home' ]]; then
  export JAVA_HOME='/Applications/Android Studio.app/Contents/jbr/Contents/Home'
fi
(cd android && ./gradlew testDebugUnitTest --tests ca.whnet.agentrouter.RouterTest) || result=1
xcodebuild -project ios/AgentRouter/AgentRouter.xcodeproj -scheme AgentRouter \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  -derivedDataPath ios/AgentRouter/build-sim CODE_SIGNING_ALLOWED=NO \
  -only-testing:AgentRouterTests/DecodingTests/testTokenOnlyRouterAuthStatusKeepsActiveTokenCount test || result=1
exit "$result"
