#!/bin/bash
# Build the menu-bar readout, wrap it in a .app bundle so macOS treats it as a proper
# accessory app (menu bar only, no Dock icon), and start it at login.
set -euo pipefail

LABEL="ca.whnet.agent-router.menubar"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
APP="$HOME/Applications/Agent Router Menu.app"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "==> building"
(cd "$REPO" && cargo build --release -p agent-router-menubar)

echo "==> bundling to $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$REPO/target/release/agent-router-menubar" "$APP/Contents/MacOS/"
cat > "$APP/Contents/Info.plist" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>            <string>Agent Router Menu</string>
  <key>CFBundleDisplayName</key>     <string>Agent Router</string>
  <key>CFBundleIdentifier</key>      <string>$LABEL</string>
  <key>CFBundleExecutable</key>      <string>agent-router-menubar</string>
  <key>CFBundlePackageType</key>     <string>APPL</string>
  <key>CFBundleShortVersionString</key> <string>0.1.0</string>
  <!-- Accessory app: lives in the menu bar, never in the Dock or the app switcher. -->
  <key>LSUIElement</key>             <true/>
  <key>LSMinimumSystemVersion</key>  <string>11.0</string>
  <key>NSHighResolutionCapable</key> <true/>
</dict>
PLIST_EOF
echo "</plist>" >> "$APP/Contents/Info.plist"

# Ad-hoc sign so macOS is willing to run it without quarantine complaints.
codesign --force --sign - "$APP" >/dev/null 2>&1 || true

echo "==> writing $PLIST"
cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>            <string>$LABEL</string>
  <key>ProgramArguments</key> <array><string>$APP/Contents/MacOS/agent-router-menubar</string></array>
  <key>RunAtLoad</key>        <true/>
  <key>KeepAlive</key>        <dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key> <integer>10</integer>
</dict>
</plist>
PLIST_EOF

launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$UID" "$PLIST"
launchctl enable "gui/$UID/$LABEL"
echo "==> running — look at the right-hand side of your menu bar"
echo "    remove with: launchctl bootout gui/$UID/$LABEL && rm -rf \"$APP\" \"$PLIST\""
