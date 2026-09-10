#!/bin/bash
# Remove the Agent Router LaunchAgent. Leaves ~/.agent-router (database, tokens,
# logs) alone; delete that directory yourself if you want it gone.
set -euo pipefail
LABEL="ca.whnet.agent-router"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
rm -f "$PLIST"
echo "removed $LABEL"
echo "data kept at $HOME/.agent-router (delete it yourself if you want it gone)"
