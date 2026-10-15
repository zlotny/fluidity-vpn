#!/bin/sh
set -u
[ "$(id -u)" -eq 0 ] || { echo "Run as root" >&2; exit 1; }
launchctl bootout system/org.fluidityvpn.daemon 2>/dev/null || true
rm -f /Library/LaunchDaemons/org.fluidityvpn.daemon.plist /var/run/fluidityd.sock
rm -rf "/Library/Application Support/Fluidity/fluidityd" "/Library/Application Support/Fluidity/openvpn" "/Library/Application Support/Fluidity/lyrebird" \
  "/Library/Application Support/Fluidity/uninstall.sh" /var/run/fluidity
[ -L /usr/local/bin/fluidity ] && rm -f /usr/local/bin/fluidity
exit 0
