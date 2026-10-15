#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
docker build -q -t fluidity-linux "$root/linux" >/dev/null
exec docker run --rm --cap-add NET_ADMIN --device /dev/net/tun \
  -v "$root":/src \
  -v fluidity-linux-target:/target \
  -v fluidity-e2e-cargo:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  -e SHOTS="${SHOTS:-/tmp/shots}" \
  fluidity-linux sh /src/tests/e2e/linux-gui.sh
