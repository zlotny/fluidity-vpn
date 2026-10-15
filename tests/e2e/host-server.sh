#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:?output directory}
mkdir -p "$out"
docker build -q -t fluidity-e2e "$root/tests/e2e" >/dev/null
docker rm -f fluidity-test-server >/dev/null 2>&1 || true
exec docker run --rm --name fluidity-test-server --cap-add NET_ADMIN --device /dev/net/tun \
  -p 11194:1194/udp -p 12443:2443/tcp -e OBFS_BIND=0.0.0.0 \
  -v "$root":/src:ro -v "$out":/out fluidity-e2e sh /src/tests/e2e/serve.sh
