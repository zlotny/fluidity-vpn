<p align="center">
  <img src="linux/data/icons/org.fluidityvpn.Fluidity.svg" width="96" alt="">
</p>

<h1 align="center">Fluidity</h1>

<p align="center">
  A free and open source OpenVPN client for macOS, Windows and Linux.<br>
  <sub>A reimplementation of Viscosity.</sub>
</p>

<p align="center">
  <a href="https://github.com/zlotny/fluidity-vpn/actions/workflows/ci.yml"><img src="https://github.com/zlotny/fluidity-vpn/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-0BSD-blue.svg" alt="License: 0BSD"></a>
</p>

Fluidity sits in your menu bar or system tray and manages all your OpenVPN connections. Connect with one click, see live traffic, and let Fluidity handle DNS, proxies, credentials and reconnects. A small privileged service does the work that needs admin rights, so the app itself runs as you.

## Features

**Connections**
- Menu bar app on macOS, tray app on Windows and Linux, with a main window for details.
- Folders, drag and drop ordering, connect all, disconnect all.
- Live status, traffic statistics, traffic graphs and the OpenVPN log per connection.
- Data usage history per connection for today, the last 7 days and the last 30 days.
- A connection check that shows your public IP, its location and the latency to the server (macOS).
- Notifications when connections go up or down.

**Editing and import**
- A connection editor for every common OpenVPN option: servers, protocols, ciphers, TLS, routes, compression, keepalive and advanced directives.
- Import `.ovpn` and `.conf` files, zip and tar.gz bundles, and other clients' bundles. Drag files onto the window to import.
- Export connections as `.fvpnz` archives to share them.

**Authentication**
- Username and password, with secure storage in the macOS Keychain, Windows Credential Manager or the Secret Service on Linux.
- Static and dynamic challenges (OTP), and web-based authentication.
- Certificates from files, PKCS#12, PKCS#11 smart cards and tokens, and macOS Keychain identities.

**Networking**
- Kill switch: blocks internet traffic outside the VPN, and keeps it blocked if the connection drops. Local network access is optional.
- Full and split DNS, with automatic restore after a crash.
- Proxy settings pushed to the system while connected (macOS).
- IPv6 leak blocking when the tunnel has no IPv6.
- Connect through HTTP and SOCKS proxies with authentication, or the system proxy.
- obfs4 obfuscation for networks that block VPN traffic.

**Behaviour**
- Reconnect after sleep and on network change. Disconnect when idle.
- Connect on launch and launch at login.
- Connect automatically on untrusted Wi-Fi, and disconnect on trusted networks.
- Choose the OpenVPN version per connection.

**Automation**
- Scripts before connect, after connect and after disconnect. A before-connect script can block the connection or supply credentials.
- Custom menu items that run your own scripts.
- The `fluidity` command-line tool, and AppleScript on macOS.
- `fluidity://` links that connect, disconnect or toggle a connection.
- A global keyboard shortcut that connects or disconnects (macOS).

**Enterprise**
- Preconfigured deployments with connections, preferences and menu items.
- Managed preferences that users cannot change.
- A `.pkg` installer for macOS, an MSI for Windows, and deb and rpm packages for Linux.

## Install

Download the package for your system from the [Releases](https://github.com/zlotny/fluidity-vpn/releases) page.

| System | Package | Notes |
|---|---|---|
| macOS 13 or later | `Fluidity-<version>.pkg` | Includes OpenVPN and obfs4. Installs the app and its helper service. |
| Windows 10 or later | `Fluidity-<version>-x64.msi` | Needs [OpenVPN 2.6 or later](https://openvpn.net/community-downloads/). Installs the app, the service and the CLI. |
| Ubuntu 24.04, Debian 13 | `fluidity_<version>_<arch>.deb` | `sudo apt install ./fluidity_*.deb` |
| Fedora 42 | `fluidity-<version>.<arch>.rpm` | `sudo dnf install ./fluidity-*.rpm` |

On Linux the tray icon needs a StatusNotifierItem host. KDE has one. On GNOME, install the AppIndicator extension.

## Usage

Open Fluidity, then import a connection file from your VPN provider: click **+**, choose **Import Connection**, or drag the file onto the window. Click a connection in the menu or tray to connect. Fluidity asks for credentials when it needs them, and can save them for next time.

### Command line

On macOS, click **Install Command-Line Tool…** in Preferences > Advanced first.

```sh
fluidity import client.ovpn
fluidity connect "My VPN"
fluidity status
fluidity usage "My VPN"
fluidity disconnect --all
fluidity --help
```

### Links

Open these links from a browser, a launcher, a shortcut or another app:

| Link | Action |
|---|---|
| `fluidity://connect/My%20VPN` | Connect "My VPN". Without a name, connect the last used connection. |
| `fluidity://disconnect/My%20VPN` | Disconnect "My VPN". Without a name, disconnect all connections. |
| `fluidity://toggle/My%20VPN` | Connect or disconnect "My VPN". Without a name, connect the last used connection, or disconnect all. |

### AppleScript

```applescript
tell application "Fluidity"
    connect "My VPN"
    get {name, state} of every connection
end tell
```

### Scripts and menu items

Each connection can run a script before connect, after connect and after disconnect. Scripts get the connection details in environment variables such as `FLUIDITY_DISPLAYNAME`, `FLUIDITY_IPV4ADDRESS` and `FLUIDITY_INTERFACE`. A before-connect script can print `FluidityNoConnect` to stop the connection, or `FluidityUsername:<name>` and `FluidityPassword:<password>` to supply credentials.

Put `<Name>.fluiditymenuitem` files (a script, or a folder with a `script` file and an optional `icon.png`) in the `Menu Items` folder of the Fluidity data folder. They appear in the menu.

## Enterprise deployment

Put a deployment folder at one of these paths:

| System | Path |
|---|---|
| macOS | `/Library/Application Support/Fluidity/Deploy` or `Fluidity.app/Contents/Resources/Deploy` |
| Windows | `C:\ProgramData\Fluidity\Deploy` |
| Linux | `/etc/fluidity/deploy` |

| Item | Purpose |
|---|---|
| `deploy.json` | `{"version": "1", "connections": "replace"}`. Modes: `append`, `replace`, `overwrite`, `prompt`. |
| `Connections/` | `.ovpn`, `.conf`, `.fvpn`, `.fvpnz` and other files to import. |
| `preferences.json` | Initial preferences, applied once. |
| `Menu Items/` | `.fluiditymenuitem` files. |

The app applies each deployment version once per user. `fluidity deploy` applies it from the command line. Preferences in `managed.json` next to the `Deploy` folder (`/etc/fluidity/managed.json` on Linux) are enforced on every start.

## Build from source

You need Rust stable. The core, the service and the CLI build on every system:

```sh
cargo test --workspace
cargo build --release -p fluidityd -p fluidity-cli
```

**macOS** needs the Xcode Command Line Tools.

```sh
macos/build-app.sh          # dist/Fluidity.app
PKG=1 macos/build-app.sh    # also dist/Fluidity-<version>.pkg
```

`build-app.sh` reads `TARGETS` (Rust target triples), `OPENVPN_DIR` (OpenVPN builds to bundle, one folder per version), `SIGN_IDENTITY` and `INSTALLER_IDENTITY`.

**Linux** builds run in Docker, based on Ubuntu 24.04. The app needs GTK 4.14 and libadwaita 1.5.

```sh
linux/build.sh "cargo build --release"     # GTK app
linux/build.sh "sh /src/linux/package.sh"  # dist/linux/*.deb and *.rpm
```

**Windows** needs the MSVC toolchain, GTK 4 and libadwaita from [gvsbuild](https://github.com/wingtk/gvsbuild), and WiX 6.

```powershell
gvsbuild build gtk4 libadwaita librsvg adwaita-icon-theme
dotnet tool install --global wix --version 6.0.2
wix extension add -g WixToolset.UI.wixext/6.0.2
windows\build-msi.ps1      # dist\windows\Fluidity-<version>-x64.msi
```

**Tests.** `tests/e2e/run.sh` connects to a real OpenVPN server in Docker. `tests/e2e/run-linux-gui.sh` drives the Linux app under Xvfb.

### Layout

| Path | Contents |
|---|---|
| `crates/fluidityd` | Privileged service: runs OpenVPN, changes DNS, proxies and firewall rules. |
| `crates/fluidity-cli` | The `fluidity` command-line tool. |
| `crates/fluidity-core` | Connection store, import, export, preferences, deployment. |
| `crates/fluidity-config` | OpenVPN config parser and writer. |
| `crates/fluidity-ipc` | Protocol between the apps and the service. |
| `crates/fluidity-mgmt` | OpenVPN management interface client. |
| `crates/fluidity-ffi` | C interface for the macOS app. |
| `macos/` | macOS app (Swift, SwiftUI). |
| `linux/` | GTK 4 and libadwaita app for Linux and Windows, Linux packaging. |
| `windows/` | Windows installer (WiX). |
| `tests/e2e/` | End-to-end tests. |

## License

[0BSD](LICENSE). Bundled third-party programs keep their own licenses: OpenVPN (GPL-2.0) and lyrebird (BSD-3-Clause).
