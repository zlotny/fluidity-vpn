#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
docker build -q -t fluidity-e2e "$root/tests/e2e" >/dev/null
exec docker run --rm --cap-add NET_ADMIN --device /dev/net/tun \
  -v "$root":/src:ro \
  -v fluidity-e2e-target:/target \
  -v fluidity-e2e-cargo:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  fluidity-e2e sh /src/tests/e2e/inside.sh
