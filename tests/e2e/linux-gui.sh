#!/bin/sh
set -eu
shots=${SHOTS:-/tmp/shots}
mkdir -p "$shots"
cd /src && cargo build -q -p fluidityd -p fluidity-cli
cd /src/linux && cargo build -q
bin=$CARGO_TARGET_DIR/debug
export PATH="$bin:$PATH"

. /src/tests/e2e/server-setup.sh

export HOME=/tmp/home FLUIDITY_HOME=/tmp/home/fluidity
mkdir -p /work && cd /work
cp $pki/ca.crt $pki/client.crt $pki/client.key $pki/tc.key .
cat > office.ovpn <<EOS
#fluidity name "Office"
client
dev tun
proto udp
remote 127.0.0.1 1194
nobind
ca ca.crt
cert client.crt
key client.key
tls-crypt tc.key
auth-user-pass
remote-cert-tls server
verb 3
EOS
sed 's/"Office"/"Lab"/' office.ovpn > lab.ovpn
fluidity import office.ovpn lab.ovpn >/dev/null

fluidityd > /tmp/fluidityd.log 2>&1 &
for _ in $(seq 50); do [ -S /var/run/fluidityd.sock ] && break; sleep 0.1; done

Xvfb :99 -screen 0 1100x720x24 >/dev/null 2>&1 &
export DISPLAY=:99 GDK_BACKEND=x11 GSK_RENDERER=cairo NO_AT_BRIDGE=1
sleep 1
openbox >/dev/null 2>&1 &

pass=0
fail=0
check() {
  if eval "$2"; then echo "PASS $1"; pass=$((pass + 1)); else echo "FAIL $1"; fail=$((fail + 1)); fi
}
shot() { sleep "${2:-1}"; import -window root "$shots/$1.png"; }
key() { xdotool key --clearmodifiers "$@"; }
wait_for() {
  for _ in $(seq 100); do eval "$1" && return 0; sleep 0.2; done
  return 1
}

dbus-run-session -- sh -c 'fluidity-gtk > /tmp/gui.log 2>&1' &
check "window opens" "wait_for 'xdotool search --name \"^Fluidity\$\" >/dev/null'"
win=$(xdotool search --name '^Fluidity$' | head -n 1)
xdotool windowactivate --sync "$win" 2>/dev/null || true
shot main 2

key ctrl+Return
check "credentials prompt" "wait_for 'fluidity status | grep -qE \"Connecting|Authenticating\"'"
shot prompt 2
xdotool type --delay 20 alice
key Tab
xdotool type --delay 20 secret
key Return
check "connects from the GUI" "wait_for 'fluidity status | grep -q Connected'"
ping -c 3 -i 0.3 10.99.0.1 >/dev/null 2>&1 || true
shot connected 3
key alt+3
check "usage page counts traffic" "wait_for 'fluidity usage Office | grep -qv \"0 B down\"'"
shot usage 2
key alt+1

key ctrl+e
shot editor 2
key Escape
sleep 1
key ctrl+comma
shot preferences 2
key Escape
sleep 1

key ctrl+Return
check "disconnects from the GUI" "wait_for '! fluidity status | grep -q Connected'"
shot disconnected 2

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ] || { grep -B2 -A12 -m1 panicked /tmp/gui.log || tail -n 30 /tmp/gui.log; tail -n 20 /tmp/fluidityd.log; exit 1; }
