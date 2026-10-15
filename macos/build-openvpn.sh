#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
work=${WORK:-$root/target/openvpn-build}
src=${SOURCES:-$root/target/openvpn-src}
out=${OUT:-$root/target/openvpn-dist}
archs=${ARCHS:-arm64 x86_64}
jobs=$(sysctl -n hw.ncpu)
export MACOSX_DEPLOYMENT_TARGET=13.0
export GIT_DIR=/nonexistent GIT_CEILING_DIRECTORIES=/
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct -- "$here/openvpn-sources.txt" 2>/dev/null || echo 0)}
export ZERO_AR_DATE=1

mkdir -p "$src" "$work" "$out"
while read -r name url sum; do
  case "$name" in ''|'#'*) continue ;; esac
  file="$src/$(basename "$url")"
  [ -f "$file" ] || curl -fsSL -o "$file" "$url"
  echo "$sum  $file" | shasum -a 256 -c - >/dev/null || { echo "Checksum mismatch: $file" >&2; exit 1; }
done < "$here/openvpn-sources.txt"

version_of() { sed -n "s/^$1 .*\/[a-z0-9-]*-\([0-9.]*\)\.tar\..*/\1/p" "$here/openvpn-sources.txt"; }
unpack() {
  rm -rf "$2" && mkdir -p "$2"
  tar -xf "$src/$1" -C "$2" --strip-components 1
}

build_deps() {
  arch=$1
  prefix="$work/$arch/prefix"
  host=$( [ "$arch" = arm64 ] && echo aarch64-apple-darwin || echo x86_64-apple-darwin )
  flags="-arch $arch -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET -O2"
  [ -f "$prefix/.done" ] && return
  mkdir -p "$prefix"

  d="$work/$arch/openssl"; unpack "openssl-$(version_of openssl).tar.gz" "$d"
  target=$( [ "$arch" = arm64 ] && echo darwin64-arm64-cc || echo darwin64-x86_64-cc )
  (cd "$d" && ./Configure "$target" no-shared no-tests no-docs no-apps --prefix="$prefix" --libdir=lib \
    "-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET" >/dev/null && make -j"$jobs" >/dev/null && make install_sw >/dev/null)

  d="$work/$arch/lzo"; unpack "lzo-$(version_of lzo).tar.gz" "$d"
  (cd "$d" && CFLAGS="$flags" ./configure --host="$host" --prefix="$prefix" --disable-shared --enable-static >/dev/null \
    && make -j"$jobs" >/dev/null && make install >/dev/null)

  d="$work/$arch/lz4"; unpack "lz4-$(version_of lz4).tar.gz" "$d"
  (cd "$d/lib" && make -j"$jobs" CFLAGS="$flags" liblz4.a >/dev/null \
    && mkdir -p "$prefix/lib" "$prefix/include" "$prefix/lib/pkgconfig" \
    && cp liblz4.a "$prefix/lib/" && cp lz4.h lz4hc.h lz4frame.h "$prefix/include/" \
    && printf 'prefix=%s\nlibdir=${prefix}/lib\nincludedir=${prefix}/include\nName: lz4\nDescription: lz4\nVersion: %s\nLibs: -L${libdir} -llz4\nCflags: -I${includedir}\n' \
      "$prefix" "$(version_of lz4)" > "$prefix/lib/pkgconfig/liblz4.pc")

  d="$work/$arch/pkcs11-helper"; unpack "pkcs11-helper-$(version_of pkcs11-helper).tar.bz2" "$d"
  (cd "$d" && PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" CFLAGS="$flags" ./configure --host="$host" --prefix="$prefix" \
    --disable-shared --enable-static --disable-crypto-engine-gnutls --disable-crypto-engine-nss \
    --disable-crypto-engine-mbedtls >/dev/null && make -j"$jobs" >/dev/null && make install >/dev/null)
  touch "$prefix/.done"
}

build_openvpn() {
  arch=$1
  version=$2
  prefix="$work/$arch/prefix"
  host=$( [ "$arch" = arm64 ] && echo aarch64-apple-darwin || echo x86_64-apple-darwin )
  d="$work/$arch/openvpn-$version"; unpack "openvpn-$version.tar.gz" "$d"
  (cd "$d" && PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" \
    CFLAGS="-arch $arch -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET -O2 -I$prefix/include" \
    LDFLAGS="-arch $arch -L$prefix/lib" \
    OPENSSL_CFLAGS="-I$prefix/include" OPENSSL_LIBS="$prefix/lib/libssl.a $prefix/lib/libcrypto.a" \
    LZO_CFLAGS="-I$prefix/include" LZO_LIBS="$prefix/lib/liblzo2.a" \
    LZ4_CFLAGS="-I$prefix/include" LZ4_LIBS="$prefix/lib/liblz4.a" \
    PKCS11_HELPER_CFLAGS="-I$prefix/include" PKCS11_HELPER_LIBS="$prefix/lib/libpkcs11-helper.a $prefix/lib/libcrypto.a" \
    ./configure --host="$host" --disable-plugins --disable-debug --enable-pkcs11 --enable-lzo --enable-lz4 >/dev/null \
    && make -j"$jobs" >/dev/null)
  cp "$d/src/openvpn/openvpn" "$work/$arch/openvpn-$version.bin"
}

for arch in $archs; do build_deps "$arch"; done
for version in $(sed -n 's/^openvpn-[0-9.]* .*\/openvpn-\([0-9.]*\)\.tar\.gz .*/\1/p' "$here/openvpn-sources.txt"); do
  for arch in $archs; do build_openvpn "$arch" "$version"; done
  mkdir -p "$out/$version"
  set --
  for arch in $archs; do set -- "$@" "$work/$arch/openvpn-$version.bin"; done
  lipo -create "$@" -output "$out/$version/openvpn"
  for dylib in $(otool -L "$out/$version/openvpn" | awk '/^[[:space:]]/ {print $1}'); do
    case "$dylib" in /usr/lib/*|/System/*) ;; *) echo "Unexpected dynamic library $dylib" >&2; exit 1 ;; esac
  done
  echo "$out/$version/openvpn"
done
