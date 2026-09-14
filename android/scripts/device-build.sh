#!/bin/sh
# Build, install and launch the app on whatever Android device adb can see.
#
#   scripts/device-build.sh            # build, install, launch
#   scripts/device-build.sh test       # unit tests only
#   DEVICE=<serial> scripts/device-build.sh
#
# Firebase is optional: without app/google-services.json the app still builds and runs,
# and says so under Settings → Notifications. See docs/google-push-requirements.md.
set -e
cd "$(dirname "$0")/.."

# Gradle needs a JDK 17+. `/usr/libexec/java_home` finds none on this Mac even though
# `java` answers, so fall back to the one Android Studio ships.
if [ -z "${JAVA_HOME:-}" ]; then
  STUDIO="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
  [ -d "$STUDIO" ] && JAVA_HOME="$STUDIO"
  export JAVA_HOME
fi
: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
export ANDROID_HOME
[ -f local.properties ] || echo "sdk.dir=$ANDROID_HOME" > local.properties

if [ "${1:-}" = "test" ]; then
  exec ./gradlew :app:testDebugUnitTest
fi

./gradlew :app:assembleDebug

ADB="$ANDROID_HOME/platform-tools/adb"
TARGET=""
[ -n "${DEVICE:-}" ] && TARGET="-s $DEVICE"
if ! $ADB $TARGET get-state >/dev/null 2>&1; then
  echo "No device. The APK is at app/build/outputs/apk/debug/app-debug.apk."
  exit 0
fi
# shellcheck disable=SC2086
$ADB $TARGET install -r app/build/outputs/apk/debug/app-debug.apk
# shellcheck disable=SC2086
$ADB $TARGET shell am start -n ca.whnet.agentrouter/.MainActivity >/dev/null
echo "installed and launched"
