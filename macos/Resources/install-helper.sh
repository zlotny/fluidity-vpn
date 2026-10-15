#!/bin/sh
set -eu
resources=$1
dest="/Library/Application Support/Fluidity"
plist=/Library/LaunchDaemons/org.fluidityvpn.daemon.plist
[ "$(id -u)" -eq 0 ] || { echo "Run as root" >&2; exit 1; }
[ -x "$resources/fluidityd" ] || { echo "Missing $resources/fluidityd" >&2; exit 1; }
launchctl bootout system/org.fluidityvpn.daemon 2>/dev/null || true
mkdir -p "$dest" /Library/Logs/Fluidity
rm -rf "$dest/openvpn.new"
if [ -d "$resources/openvpn" ]; then
  cp -R "$resources/openvpn" "$dest/openvpn.new"
  rm -rf "$dest/openvpn"
  mv "$dest/openvpn.new" "$dest/openvpn"
fi
if [ -x "$resources/lyrebird" ]; then
  cp "$resources/lyrebird" "$dest/lyrebird.new"
  mv "$dest/lyrebird.new" "$dest/lyrebird"
fi
cp "$resources/fluidityd" "$dest/fluidityd.new"
mv "$dest/fluidityd.new" "$dest/fluidityd"
cp "$resources/uninstall-helper.sh" "$dest/uninstall.sh"
cp "$resources/org.fluidityvpn.daemon.plist" "$plist"
chown -R root:wheel "$dest" "$plist"
chmod -R go-w "$dest"
chmod 755 "$dest" "$dest/fluidityd" "$dest/uninstall.sh"
chmod 644 "$plist"
launchctl bootstrap system "$plist"
launchctl enable system/org.fluidityvpn.daemon
for _ in $(seq 50); do [ -S /var/run/fluidityd.sock ] && exit 0; sleep 0.1; done
echo "The helper did not start. See /Library/Logs/Fluidity/fluidityd.log" >&2
exit 1
