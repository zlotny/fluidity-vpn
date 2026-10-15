#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
dist=${DIST:-$root/dist}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
build=${BUILD_NUMBER:-$(git -C "$root" rev-list --count HEAD 2>/dev/null || echo 1)}
identity=${SIGN_IDENTITY:--}
targets=${TARGETS:-$(uname -m | sed 's/arm64/aarch64/')-apple-darwin}
export MACOSX_DEPLOYMENT_TARGET=13.0
export PATH="$HOME/.cargo/bin:$PATH"

libdir="$root/target/macos-universal"
mkdir -p "$libdir"
for target in $targets; do
  cargo build --manifest-path "$root/Cargo.toml" --release --target "$target" \
    -p fluidity-ffi -p fluidityd -p fluidity-cli
done
merge() {
  out=$1
  shift
  set -- $(for t in $targets; do echo "$root/target/$t/release/$1"; done)
  if [ $# -eq 1 ]; then cp "$1" "$out"; else lipo -create "$@" -output "$out"; fi
}
merge "$libdir/libfluidity_ffi.a" libfluidity_ffi.a
merge "$libdir/fluidityd" fluidityd
merge "$libdir/fluidity" fluidity

arch_flags=""
for t in $targets; do arch_flags="$arch_flags --arch ${t%%-*}"; done
arch_flags=$(echo "$arch_flags" | sed 's/aarch64/arm64/g')
(cd "$here" && FLUIDITY_LIB_DIR="$libdir" swift build -c release $arch_flags)
bin=$(cd "$here" && FLUIDITY_LIB_DIR="$libdir" swift build -c release $arch_flags --show-bin-path)

app="$dist/Fluidity.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers" "$app/Contents/Resources"
cp "$bin/Fluidity" "$app/Contents/MacOS/Fluidity"
cp "$libdir/fluidity" "$app/Contents/Helpers/fluidity"
cp "$libdir/fluidityd" "$app/Contents/Resources/fluidityd"
cp "$here/Resources/Fluidity.sdef" "$here/Resources/org.fluidityvpn.daemon.plist" \
  "$here/Resources/install-helper.sh" "$here/Resources/uninstall-helper.sh" "$app/Contents/Resources/"

if [ -n "${OPENVPN_DIR:-}" ]; then
  for dir in "$OPENVPN_DIR"/*/; do
    v=$(basename "$dir")
    mkdir -p "$app/Contents/Resources/openvpn/$v"
    cp -R "$dir". "$app/Contents/Resources/openvpn/$v/"
  done
fi

iconset=$(mktemp -d)/AppIcon.iconset
"$app/Contents/MacOS/Fluidity" --render-iconset "$iconset"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$iconset")"

sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/$build/g" "$here/Resources/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

sign() {
  if [ "$identity" = "-" ]; then
    codesign --force --sign - "$@"
  else
    codesign --force --options runtime --timestamp --sign "$identity" "$@"
  fi
}
find "$app/Contents/Resources/openvpn" -type f -perm +111 2>/dev/null | while read -r f; do sign "$f"; done
sign "$app/Contents/Resources/fluidityd"
sign "$app/Contents/Helpers/fluidity"
sign --entitlements "$here/Resources/Fluidity.entitlements" "$app"
codesign --verify --strict "$app"
echo "$app"

if [ "${PKG:-0}" = 1 ]; then
  pkgroot=$(mktemp -d)
  mkdir -p "$pkgroot/root/Applications" "$pkgroot/scripts"
  cp -R "$app" "$pkgroot/root/Applications/"
  cp "$here/Resources/postinstall" "$pkgroot/scripts/postinstall"
  chmod +x "$pkgroot/scripts/postinstall"
  pkgbuild --root "$pkgroot/root" --scripts "$pkgroot/scripts" --identifier org.fluidityvpn.Fluidity.pkg \
    --version "$version" --install-location / "$pkgroot/Fluidity.pkg"
  if [ -n "${INSTALLER_IDENTITY:-}" ]; then
    productbuild --package "$pkgroot/Fluidity.pkg" --sign "$INSTALLER_IDENTITY" "$dist/Fluidity-$version.pkg"
  else
    productbuild --package "$pkgroot/Fluidity.pkg" "$dist/Fluidity-$version.pkg"
  fi
  rm -rf "$pkgroot"
  echo "$dist/Fluidity-$version.pkg"
fi
