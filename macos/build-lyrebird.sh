#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
version=${LYREBIRD_VERSION:-0.9.0}
commit=${LYREBIRD_COMMIT:-89d54872b176f936232ff39f7a8189b77d83d65e}
work=${WORK:-$root/target/lyrebird-build}
out=${OUT:-$root/target/lyrebird-dist}
repo=https://gitlab.torproject.org/tpo/anti-censorship/pluggable-transports/lyrebird.git
mkdir -p "$work" "$out"
if [ ! -d "$work/src/.git" ]; then git clone -q "$repo" "$work/src"; fi
git -C "$work/src" fetch -q --tags
git -C "$work/src" checkout -q "$commit"
[ "$(git -C "$work/src" rev-parse HEAD)" = "$commit" ] || { echo "Commit mismatch" >&2; exit 1; }
set --
for arch in arm64 amd64; do
  (cd "$work/src" && CGO_ENABLED=0 GOOS=darwin GOARCH=$arch go build -trimpath -buildvcs=false \
    -ldflags "-s -w -X main.lyrebirdVersion=$version" -o "$work/lyrebird-$arch" ./cmd/lyrebird)
  set -- "$@" "$work/lyrebird-$arch"
done
lipo -create "$@" -output "$out/lyrebird"
echo "$out/lyrebird"
