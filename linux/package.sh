#!/bin/sh
set -eu
out=${OUT:-/src/dist/linux}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' /src/Cargo.toml | head -n 1)
arch=$(dpkg --print-architecture)
cd /src && cargo build -q --release -p fluidityd -p fluidity-cli
cd /src/linux && cargo build -q --release
bin=$CARGO_TARGET_DIR/release
data=/src/linux/data
strip "$bin/fluidityd" "$bin/fluidity" "$bin/fluidity-gtk"

stage=$(mktemp -d)
root=$stage/root
install -Dm755 "$bin/fluidityd" "$root/usr/sbin/fluidityd"
install -Dm755 "$bin/fluidity" "$root/usr/bin/fluidity"
install -Dm755 "$bin/fluidity-gtk" "$root/usr/bin/fluidity-gtk"
install -Dm644 "$data/fluidityd.service" "$root/usr/lib/systemd/system/fluidityd.service"
install -Dm644 "$data/org.fluidityvpn.Fluidity.desktop" "$root/usr/share/applications/org.fluidityvpn.Fluidity.desktop"
install -Dm644 "$data/org.fluidityvpn.Fluidity.metainfo.xml" "$root/usr/share/metainfo/org.fluidityvpn.Fluidity.metainfo.xml"
install -Dm644 "$data/org.fluidityvpn.Fluidity.mime.xml" "$root/usr/share/mime/packages/org.fluidityvpn.Fluidity.xml"
install -Dm644 "$data/icons/org.fluidityvpn.Fluidity.svg" "$root/usr/share/icons/hicolor/scalable/apps/org.fluidityvpn.Fluidity.svg"
install -Dm644 "$data/icons/org.fluidityvpn.Fluidity-symbolic.svg" "$root/usr/share/icons/hicolor/symbolic/apps/org.fluidityvpn.Fluidity-symbolic.svg"
for size in 48 128 256; do
  dir="$root/usr/share/icons/hicolor/${size}x${size}/apps"
  mkdir -p "$dir"
  rsvg-convert -w "$size" -h "$size" -o "$dir/org.fluidityvpn.Fluidity.png" "$data/icons/org.fluidityvpn.Fluidity.svg"
done
install -Dm644 /src/LICENSE "$root/usr/share/doc/fluidity/copyright"
mkdir -p "$out"

deb=$stage/deb
cp -a "$root" "$deb"
mkdir -p "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<EOC
Package: fluidity
Version: $version
Architecture: $arch
Maintainer: The Fluidity Contributors <fluidity@users.noreply.github.com>
Section: net
Priority: optional
Homepage: https://github.com/fluidity-vpn/fluidity
Depends: openvpn (>= 2.5), libgtk-4-1 (>= 4.14), libadwaita-1-0 (>= 1.5), libdbus-1-3, libc6
Recommends: obfs4proxy | lyrebird
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Description: Free and open source OpenVPN client
 Fluidity manages OpenVPN connections from a window and the system tray.
 It supports folders, saved passwords, scripts, proxies, traffic
 obfuscation and preconfigured deployments.
EOC
cat > "$deb/DEBIAN/postinst" <<'EOC'
#!/bin/sh
set -e
if [ "$1" = configure ] && [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
  systemctl enable --now fluidityd.service || true
fi
EOC
cat > "$deb/DEBIAN/prerm" <<'EOC'
#!/bin/sh
set -e
if [ -d /run/systemd/system ] && [ "$1" = remove ]; then
  systemctl disable --now fluidityd.service || true
fi
EOC
cat > "$deb/DEBIAN/postrm" <<'EOC'
#!/bin/sh
set -e
if [ "$1" = purge ]; then
  rm -rf /var/lib/fluidity
fi
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
fi
EOC
chmod 755 "$deb/DEBIAN/postinst" "$deb/DEBIAN/prerm" "$deb/DEBIAN/postrm"
dpkg-deb --root-owner-group -Zxz --build "$deb" "$out/fluidity_${version}_${arch}.deb" >/dev/null

rpmarch=$(uname -m)
top=$stage/rpm
mkdir -p "$top/SPECS"
cat > "$top/SPECS/fluidity.spec" <<EOC
Name: fluidity
Version: $version
Release: 1
Summary: Free and open source OpenVPN client
License: GPL-3.0-or-later
URL: https://github.com/fluidity-vpn/fluidity
Requires: openvpn >= 2.5
Requires: gtk4 >= 4.14
Requires: libadwaita >= 1.5
Requires: dbus-libs
AutoReqProv: no

%description
Fluidity manages OpenVPN connections from a window and the system tray.

%install
cp -a $root/. %{buildroot}/

%post
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || :
  systemctl enable --now fluidityd.service || :
fi

%preun
if [ \$1 -eq 0 ] && [ -d /run/systemd/system ]; then
  systemctl disable --now fluidityd.service || :
fi

%files
/usr/sbin/fluidityd
/usr/bin/fluidity
/usr/bin/fluidity-gtk
/usr/lib/systemd/system/fluidityd.service
/usr/share/applications/org.fluidityvpn.Fluidity.desktop
/usr/share/metainfo/org.fluidityvpn.Fluidity.metainfo.xml
/usr/share/mime/packages/org.fluidityvpn.Fluidity.xml
/usr/share/icons/hicolor/*/apps/org.fluidityvpn.Fluidity*
%license /usr/share/doc/fluidity/copyright
EOC
rpmbuild -bb --quiet --define "_topdir $top" --define "_rpmdir $out" --define "_build_id_links none" \
  --target "$rpmarch" "$top/SPECS/fluidity.spec" >/dev/null
find "$out" -name '*.rpm' -path "*/$rpmarch/*" -exec mv {} "$out/" \;
rmdir "$out/$rpmarch" 2>/dev/null || true
rm -rf "$stage"
ls -l "$out"
