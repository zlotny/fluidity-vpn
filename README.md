# Fluidity VPN

Free and open source OpenVPN client for macOS, Windows, and Linux.

## Features

- Menu bar app (macOS) and GTK app with a tray icon (Linux), with folders, live statistics, traffic graphs and logs.
- Connection editor for every common OpenVPN option. Import `.ovpn`, `.conf`, `.tblk`, zip and tar.gz bundles.
- Full and split DNS, interface proxies, IPv6 leak blocking, automatic DNS restore after a crash.
- Saved credentials in the system keychain, static and dynamic challenges, web authentication.
- Certificates from files, PKCS#12, PKCS#11 tokens or keychain identities.
- HTTP and SOCKS proxies with authentication, the system proxy, and obfs4 obfuscation.
- Reconnect on wake and on network change, idle disconnect, connect on launch.
- Scripts before connect, after connect and after disconnect. Custom menu items.
- AppleScript and the `fluidity` command-line tool.

## Components

| Path | Purpose |
|---|---|
| `crates/fluidityd` | Privileged service that runs OpenVPN and changes network settings. |
| `crates/fluidity-cli` | `fluidity` command-line tool. |
| `crates/fluidity-ffi` | C interface for native user interfaces. |
| `macos/` | macOS app (Swift). |
| `linux/` | GTK 4 and libadwaita app for Linux and Windows, systemd unit, desktop files, Linux packaging. |
| `windows/` | Windows installer (WiX). |
| `tests/e2e/` | End-to-end tests against a real OpenVPN server in Docker. |

## Build

Rust stable and, for macOS, the Xcode Command Line Tools.

```sh
cargo test --workspace
macos/build-app.sh          # dist/Fluidity.app
PKG=1 macos/build-app.sh    # also dist/Fluidity-<version>.pkg
tests/e2e/run.sh            # needs Docker
```

Linux builds run in Docker, based on Ubuntu 24.04:

```sh
linux/build.sh "cargo build --release"     # GTK app
linux/build.sh "sh /src/linux/package.sh"  # dist/linux/*.deb and *.rpm
tests/e2e/run-linux-gui.sh                 # GUI test under Xvfb
```

The packages install `fluidityd` as a systemd service, the `fluidity` CLI and the `fluidity-gtk` app. The Linux app needs GTK 4.14 and libadwaita 1.5. The tray icon needs a StatusNotifierItem host (KDE, or GNOME with the AppIndicator extension).

Windows builds run on Windows with the MSVC toolchain, GTK 4 and libadwaita from [gvsbuild](https://github.com/wingtk/gvsbuild), and WiX 6:

```powershell
gvsbuild build gtk4 libadwaita librsvg adwaita-icon-theme
dotnet tool install --global wix --version 6.0.2
wix extension add -g WixToolset.UI.wixext/6.0.2
windows\build-msi.ps1      # dist\windows\Fluidity-<version>-x64.msi
```

The MSI installs `fluidityd` as the `fluidityd` service, the `fluidity` CLI on `PATH` and the GTK app with its runtime. It needs [OpenVPN 2.6 or later](https://openvpn.net/community-downloads/) in `C:\Program Files\OpenVPN`. DNS uses NRPT rules. The CI job `windows-msi` builds it.

`build-app.sh` reads `TARGETS` (Rust target triples), `OPENVPN_DIR` (OpenVPN builds to bundle, one folder per version), `SIGN_IDENTITY` and `INSTALLER_IDENTITY`.

## Command line

```sh
fluidity import client.ovpn
fluidity connect "My VPN"
fluidity status
fluidity disconnect --all
```

## Enterprise deployment

Put a deployment folder at `/Library/Application Support/Fluidity/Deploy` (macOS), `/etc/fluidity/deploy` (Linux) or `C:\ProgramData\Fluidity\Deploy` (Windows). On macOS it can also be `Fluidity.app/Contents/Resources/Deploy`.

| Item | Purpose |
|---|---|
| `deploy.json` | `{"version": "1", "connections": "replace"}`. Modes: `append`, `replace`, `overwrite`, `prompt`. |
| `Connections/` | `.ovpn`, `.conf`, `.fvpn`, `.fvpnz` and other importable files. |
| `preferences.json` | Initial preferences, merged once. |
| `Menu Items/` | `.fluiditymenuitem` files. |

The app applies each version once per user. `fluidity deploy` applies it from the command line. Preferences in `managed.json` next to the `Deploy` folder (`/etc/fluidity/managed.json` on Linux) are enforced on every start.

## AppleScript

```applescript
tell application "Fluidity"
    connect "My VPN"
    get {name, state} of every connection
end tell
```

## License

GPL-3.0-or-later.
