#!/bin/sh
set -eu
. /src/tests/e2e/server-setup.sh
host=${CLIENT_REMOTE:-127.0.0.1}
mkdir -p /out
cp $pki/ca.crt $pki/client.crt $pki/client.key $pki/tc.key /out/
cat > /out/Fluidity-Test.ovpn <<EOS
client
dev tun
proto udp
remote $host ${CLIENT_PORT:-11194}
nobind
ca ca.crt
cert client.crt
key client.key
tls-crypt tc.key
auth-user-pass
remote-cert-tls server
verb 3
EOS
cat > /out/Fluidity-Obfs.ovpn <<EOS
#fluidity obfs obfs4
#fluidity obfskey $obfs4_cert
client
dev tun
proto tcp
remote $host ${CLIENT_OBFS_PORT:-12443}
nobind
ca ca.crt
cert client.crt
key client.key
tls-crypt tc.key
auth-user-pass
remote-cert-tls server
verb 3
EOS
echo READY
exec tail -f /tmp/server.log /tmp/server-tcp.log
