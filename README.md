# Fluidity VPN

Free and open source OpenVPN client for macOS, Windows, and Linux.

## Features

- Menu bar app with folders, live statistics, traffic graphs and logs.
- Connection editor for every common OpenVPN option. Import `.ovpn`, `.conf`, `.tblk`, zip and tar.gz bundles.
- Full and split DNS, interface proxies, IPv6 leak blocking, automatic DNS restore after a crash.
- Saved credentials in the system keychain, static and dynamic challenges, web authentication.
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
| `tests/e2e/` | End-to-end tests against a real OpenVPN server in Docker. |

## Build

Rust stable and, for macOS, the Xcode Command Line Tools.

```sh
cargo test --workspace
macos/build-app.sh          # dist/Fluidity.app
PKG=1 macos/build-app.sh    # also dist/Fluidity-<version>.pkg
tests/e2e/run.sh            # needs Docker
```

`build-app.sh` reads `TARGETS` (Rust target triples), `OPENVPN_DIR` (OpenVPN builds to bundle, one folder per version), `SIGN_IDENTITY` and `INSTALLER_IDENTITY`.

## Command line

```sh
fluidity import client.ovpn
fluidity connect "My VPN"
fluidity status
fluidity disconnect --all
```

## AppleScript

```applescript
tell application "Fluidity"
    connect "My VPN"
    get {name, state} of every connection
end tell
```

## License

GPL-3.0-or-later.
