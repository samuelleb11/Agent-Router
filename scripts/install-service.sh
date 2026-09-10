#!/bin/bash
# Install Agent Router as a macOS LaunchAgent, so it starts at login and comes back
# if it crashes. It appears in System Settings > General > Login Items under
# "Allow in the Background".
#
# Run it again after any rebuild to upgrade in place — the handover is seamless, so
# a running Claude Code session will not notice.
set -euo pipefail

LABEL="ca.whnet.agent-router"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
PREFIX="$HOME/.agent-router"
BIN="$PREFIX/bin/agent-router"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "==> building release binary"
(cd "$REPO" && cargo build --release)

mkdir -p "$PREFIX/bin" "$PREFIX/logs"

# The service runs from its own copy, so rebuilding in the repo never swaps the
# binary out from under a running process.
echo "==> installing to $BIN"
cp "$REPO/target/release/agent-router" "$BIN.new"
mv -f "$BIN.new" "$BIN"

# The router shells out to `claude` for account probing, so the agent needs a PATH
# that contains it — launchd gives a process almost nothing by default.
CLAUDE_DIR="$(dirname "$(command -v claude 2>/dev/null || echo /usr/local/bin/claude)")"

echo "==> writing $PLIST"
cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>                <string>$LABEL</string>
  <key>ProgramArguments</key>     <array><string>$BIN</string></array>
  <key>RunAtLoad</key>            <true/>

  <!-- Restart only on a crash. A clean exit is deliberate: it is what happens when a
       newer build takes the port over, and restarting then would fight the handover
       and revert to this binary. -->
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>

  <key>ThrottleInterval</key>     <integer>5</integer>
  <key>ProcessType</key>          <string>Background</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>               <string>$CLAUDE_DIR:/usr/bin:/bin:/usr/sbin:/sbin</string>
    <key>HOME</key>               <string>$HOME</string>
  </dict>
  <key>StandardOutPath</key>      <string>$PREFIX/logs/router.log</string>
  <key>StandardErrorPath</key>    <string>$PREFIX/logs/router.log</string>
</dict>
</plist>
PLIST_EOF

echo "==> (re)loading the service"
launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$UID" "$PLIST"
launchctl enable "gui/$UID/$LABEL"

printf '==> waiting for it to answer'
for _ in $(seq 1 30); do
  if curl -fsS --max-time 1 http://127.0.0.1:8787/_router/health >/dev/null 2>&1; then
    echo
    echo "==> up: $(curl -fsS http://127.0.0.1:8787/_router/health)"
    echo
    echo "Dashboard: http://127.0.0.1:8787"
    echo "Logs:      $PREFIX/logs/router.log"
    echo "Stop:      launchctl bootout gui/$UID/$LABEL"
    exit 0
  fi
  printf .
  sleep 0.5
done
echo
echo "!! it did not answer within 15s — check $PREFIX/logs/router.log" >&2
exit 1
