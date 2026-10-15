#!/bin/sh
set -eu

cd /src
cargo build -q --bins
bin=$CARGO_TARGET_DIR/debug
export PATH="$bin:$PATH"

. /src/tests/e2e/server-setup.sh

client_conf() {
  cat <<EOS
#fluidity name "$1"
#fluidity dns full
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
}
mkdir -p /work && cd /work
export FLUIDITY_HOME=/work/home
cp $pki/ca.crt $pki/client.crt $pki/client.key $pki/tc.key .
client_conf "E2E" > e2e.ovpn
{ client_conf "Unsafe"; echo "up /bin/true"; } > unsafe.ovpn
{ client_conf "Challenge"; echo 'static-challenge "Enter OTP" 1'; } > challenge.ovpn
{ client_conf "Obfs" | sed 's/^remote 127.0.0.1 1194/remote 127.0.0.1 2443/'
  echo "#fluidity obfs obfs4"; echo "#fluidity obfskey $obfs4_cert"; } > obfs.ovpn
{ client_conf "Proxy" | sed -e 's/^remote 127.0.0.1 1194/remote 127.0.0.1 1195/' -e 's/^proto udp/proto tcp-client/'
  echo "http-proxy 127.0.0.1 8888 stdin basic"; } > proxy.ovpn
printf 'Port 8888\nListen 127.0.0.1\nAllow 127.0.0.1\nBasicAuth bob hunter2\nLogFile "/tmp/tinyproxy.log"\nPidFile "/tmp/tinyproxy.pid"\n' > /tmp/tinyproxy.conf
tinyproxy -c /tmp/tinyproxy.conf

FLUIDITY_LYREBIRD=/usr/bin/obfs4proxy fluidityd --log debug > /tmp/fluidityd.log 2>&1 &
for _ in $(seq 50); do [ -S /var/run/fluidityd.sock ] && break; sleep 0.1; done

pass=0
fail=0
check() {
  if eval "$2"; then echo "PASS $1"; pass=$((pass + 1)); else echo "FAIL $1"; fail=$((fail + 1)); fi
}

fluidity import e2e.ovpn unsafe.ovpn challenge.ovpn obfs.ovpn proxy.ovpn
check "list shows imported connections" "fluidity list | grep -q E2E"

cp /etc/resolv.conf /tmp/resolv.before
check "connect with credentials" "printf 'alice\nsecret\n' | timeout 60 fluidity connect E2E"
check "status reports connected" "fluidity status | grep -q 'E2E: Connected'"
check "tunnel carries traffic" "ping -c 2 -W 2 10.99.0.1 >/dev/null"
check "full DNS applied" "grep -q 'nameserver 10.99.0.1' /etc/resolv.conf"
check "log is available" "fluidity log E2E | grep -q 'Initialization Sequence Completed'"
check "disconnect" "fluidity disconnect E2E"
for _ in $(seq 100); do fluidity status | grep -q 'E2E' || break; sleep 0.1; done
check "session removed after disconnect" "! fluidity status | grep -q 'E2E'"
check "DNS restored" "cmp -s /etc/resolv.conf /tmp/resolv.before"

check "retry after wrong password" "printf 'alice\nwrong\nalice\nsecret\n' | timeout 90 fluidity connect E2E 2>&1 | tee /tmp/retry.log | grep -q '^Connected'"
fluidity disconnect --all
for _ in $(seq 100); do fluidity status | grep -q 'No active' && break; sleep 0.1; done

check "unsafe command rejected" "! fluidity connect Unsafe 2>&1 | grep -q '^Connected'"
check "unsafe error message" "fluidity connect Unsafe 2>&1 | grep -q 'is not allowed'"

check "static challenge" "printf 'alice\nsecret\n123456\n' | timeout 60 fluidity connect Challenge"
fluidity disconnect --all
for _ in $(seq 100); do fluidity status | grep -q 'No active' && break; sleep 0.1; done

check "obfs4 obfuscation" "printf 'alice\nsecret\n' | timeout 60 fluidity connect Obfs && ping -c 2 -W 2 10.98.0.1 >/dev/null"
fluidity disconnect --all
for _ in $(seq 100); do fluidity status | grep -q 'No active' && break; sleep 0.1; done

check "HTTP proxy with authentication" "printf 'alice\nsecret\nbob\nhunter2\n' | timeout 60 fluidity connect Proxy 2>&1 | tee /tmp/proxy.log | grep -q '^Connected' && ping -c 2 -W 2 10.98.0.1 >/dev/null"
check "proxy realm prompt" "grep -q 'Proxy HTTP Proxy username' /tmp/proxy.log"
fluidity disconnect --all
for _ in $(seq 100); do fluidity status | grep -q 'No active' && break; sleep 0.1; done

gateway=$(ip route | awk '/^default/ {print $3; exit}')
printf '{"kill_switch": true, "kill_switch_allow_lan": false}' > "$FLUIDITY_HOME/preferences.json"
check "gateway reachable before kill switch" "ping -c 1 -W 2 $gateway >/dev/null"
check "connect with kill switch" "printf 'alice\nsecret\n' | timeout 60 fluidity connect E2E"
check "kill switch reported" "fluidity status | grep -q 'Kill switch: on'"
check "kill switch keeps tunnel open" "ping -c 2 -W 2 10.99.0.1 >/dev/null"
check "kill switch blocks traffic outside the tunnel" "! ping -c 1 -W 2 $gateway >/dev/null"
pkill -KILL -f 'run/fluidity/' || true
for _ in $(seq 100); do fluidity status | grep -q 'internet blocked' && break; sleep 0.1; done
check "drop leaves internet blocked" "fluidity status | grep -q 'internet blocked since E2E dropped'"
check "traffic stays blocked after drop" "! ping -c 1 -W 2 $gateway >/dev/null"
check "unblock" "fluidity unblock && ! nft list table inet fluiditykillswitch >/dev/null 2>&1"
check "traffic flows after unblock" "ping -c 1 -W 2 $gateway >/dev/null"
check "connect with kill switch again" "printf 'alice\nsecret\n' | timeout 60 fluidity connect E2E"
check "user disconnect releases kill switch" "fluidity disconnect E2E && for _ in \$(seq 50); do nft list table inet fluiditykillswitch >/dev/null 2>&1 || break; sleep 0.1; done; ! nft list table inet fluiditykillswitch >/dev/null 2>&1"
check "traffic flows after disconnect" "ping -c 1 -W 2 $gateway >/dev/null"
rm -f "$FLUIDITY_HOME/preferences.json"
for _ in $(seq 100); do fluidity status | grep -q 'No active' && break; sleep 0.1; done

check "export and reimport" "fluidity export E2E /tmp/e2e.fvpnz && fluidity import --replace /tmp/e2e.fvpnz | grep -q 'E2E'"

kill -TERM %1 2>/dev/null || pkill -TERM fluidityd
sleep 1
echo "passed=$pass failed=$fail"
if [ "$fail" -ne 0 ]; then
  echo "--- fluidityd.log"; cat /tmp/fluidityd.log
  echo "--- server.log"; head -n 150 /tmp/server.log
  exit 1
fi
