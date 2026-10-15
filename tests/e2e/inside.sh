#!/bin/sh
set -eu

cd /src
cargo build -q --bins
bin=$CARGO_TARGET_DIR/debug
export PATH="$bin:$PATH"

pki=/pki
mkdir -p $pki && cd $pki
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 \
  -subj /CN=e2e-ca -keyout ca.key -out ca.crt 2>/dev/null
issue() {
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj "/CN=$1" \
    -keyout "$1.key" -out "$1.csr" 2>/dev/null
  printf 'keyUsage=critical,digitalSignature,keyAgreement\nextendedKeyUsage=%s\n' "$2" > "$1.ext"
  openssl x509 -req -in "$1.csr" -CA ca.crt -CAkey ca.key -CAcreateserial -days 2 \
    -extfile "$1.ext" -out "$1.crt" 2>/dev/null
}
issue server serverAuth
issue client clientAuth
openvpn --genkey secret tc.key

cat > check.sh <<'EOS'
#!/bin/sh
user=$(sed -n 1p "$1"); pass=$(sed -n 2p "$1")
case "$pass" in
  SCRV1:*) pw=$(echo "$pass" | cut -d: -f2 | base64 -d); otp=$(echo "$pass" | cut -d: -f3 | base64 -d)
           [ "$user" = alice ] && [ "$pw" = secret ] && [ "$otp" = 123456 ] && exit 0 ;;
  *) [ "$user" = alice ] && [ "$pass" = secret ] && exit 0 ;;
esac
exit 1
EOS
chmod +x check.sh

cat > server.conf <<EOS
port 1194
proto udp
dev tun
topology subnet
server 10.99.0.0 255.255.255.0
ca $pki/ca.crt
cert $pki/server.crt
key $pki/server.key
dh none
tls-crypt $pki/tc.key
script-security 2
auth-user-pass-verify $pki/check.sh via-file
push "dhcp-option DNS 10.99.0.1"
push "dhcp-option DOMAIN e2e.test"
keepalive 5 30
verb 3
EOS
openvpn --config server.conf --daemon --log /tmp/server.log --writepid /tmp/server.pid

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
cp $pki/ca.crt $pki/client.crt $pki/client.key $pki/tc.key .
client_conf "E2E" > e2e.ovpn
{ client_conf "Unsafe"; echo "up /bin/true"; } > unsafe.ovpn
{ client_conf "Challenge"; echo 'static-challenge "Enter OTP" 1'; } > challenge.ovpn

fluidityd --log debug > /tmp/fluidityd.log 2>&1 &
for _ in $(seq 50); do [ -S /var/run/fluidityd.sock ] && break; sleep 0.1; done

pass=0
fail=0
check() {
  if eval "$2"; then echo "PASS $1"; pass=$((pass + 1)); else echo "FAIL $1"; fail=$((fail + 1)); fi
}

fluidity import e2e.ovpn unsafe.ovpn challenge.ovpn
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

check "export and reimport" "fluidity export E2E /tmp/e2e.fvpnz && fluidity import --replace /tmp/e2e.fvpnz | grep -q 'E2E'"

kill -TERM %1 2>/dev/null || pkill -TERM fluidityd
sleep 1
echo "passed=$pass failed=$fail"
if [ "$fail" -ne 0 ]; then
  echo "--- fluidityd.log"; cat /tmp/fluidityd.log
  echo "--- server.log"; head -n 150 /tmp/server.log
  exit 1
fi
