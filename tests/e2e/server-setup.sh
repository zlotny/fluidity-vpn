#!/bin/sh
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
sed -e 's/^port 1194/port 1195/' -e 's/^proto udp/proto tcp-server/' -e 's/^server 10.99.0.0/server 10.98.0.0/' \
  server.conf > server-tcp.conf
openvpn --config server-tcp.conf --daemon --log /tmp/server-tcp.log --writepid /tmp/server-tcp.pid
mkdir -p /pt-server
TOR_PT_MANAGED_TRANSPORT_VER=1 TOR_PT_SERVER_TRANSPORTS=obfs4 TOR_PT_SERVER_BINDADDR=obfs4-${OBFS_BIND:-127.0.0.1}:2443 \
  TOR_PT_ORPORT=127.0.0.1:1195 TOR_PT_STATE_LOCATION=/pt-server obfs4proxy > /tmp/pt-server.log 2>&1 < /dev/zero &
for _ in $(seq 50); do [ -s /pt-server/obfs4_bridgeline.txt ] && break; sleep 0.1; done
obfs4_cert=$(sed -n 's/.*cert=\([^ ]*\).*/\1/p' /pt-server/obfs4_bridgeline.txt | tail -n 1)

