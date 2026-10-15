#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
docker build -q -t fluidity-linux "$root/linux" >/dev/null
exec docker run --rm \
  -v "$root":/src \
  -v fluidity-linux-target:/target \
  -v fluidity-e2e-cargo:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target \
  -w /src/linux \
  fluidity-linux sh -c "${*:-cargo build --release}"
