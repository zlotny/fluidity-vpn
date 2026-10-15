# Fluidity VPN: Implementation Plan

Fluidity is a free and open source OpenVPN client for macOS, Windows, and Linux.

## 1. Feature specification

**Connections and editor**
- Connection list with folders. Folders share credentials and scripts.
- Editor tabs: General, Authentication, Options, Networking, Transport (proxy, obfuscation), Advanced (scripts, raw OpenVPN commands).
- Multiple remotes (`server:port:proto`), UDP/TCP, IPv4/IPv6/auto, tun/tap, DHCP for tap, OpenVPN compatibility mode (2.3 to 2.6, latest).
- Simultaneous connections. Import, export, duplicate.
- Import: `.ovpn`, `.conf`, connection bundle folders, zip and tar.gz archives, Tunnelblick `.tblk`.

**Authentication**
- Username/password, PKI, PKCS#12, static key.
- PKCS#11 tokens and smartcards.
- OS certificate store identities (macOS Keychain, Windows certificate store).
- Static challenge (`static-challenge`, `#fluidity static-challenge-password`, `#fluidity static-challenge-acknowledge`).
- Dynamic challenge (`CRV1`). OTP, TOTP, Duo, YubiKey OTP.
- FIDO U2F/FIDO2 (libfido2).
- Web authentication (SSO/SAML) via `WEB_AUTH`/`OPEN_URL`.
- Credentials in the OS store (Keychain, Credential Manager, Secret Service). Clear saved credentials.

**Networking**
- DNS modes: Automatic, Full, Split, Disabled. Server can push the mode. Ignore pushed DNS. WINS.
- Routes: automatic, all traffic, all IPv4, all IPv6, custom routes, default gateway.
- IPv6 leak protection (block IPv6 on IPv4-only tunnels).
- Proxy to reach the server: HTTP, SOCKS, system-wide, with auth.
- Proxy settings on the VPN interface: PAC, HTTP, HTTPS, SOCKS.
- Obfuscation: obfs2, obfs3, obfs4, ScrambleSuit via lyrebird.
- Shaper, fragment, tun MTU, inactivity timeout.
- Server selection and fallback.

**Behaviour**
- Reconnect on drop, on wake, and when a network becomes available.
- Disconnect after inactivity.
- Disable Time Machine while connected (macOS).
- Connect at app launch per connection. Start at login.
- Reset network interfaces on disconnect.
- Unsafe OpenVPN command filter. Allowing unsafe commands needs admin rights.

**Interface**
- Status icon with menu: per-connection state, quick stats, traffic graph.
- Icon sets. Address display (IPv4/IPv6). Traffic display (rates or totals). Graph style.
- Details window: status, client IP, duration, traffic graph, totals, log. Always-on-top option.
- Notifications on state changes. Welcome window on first run.

**Automation**
- Scripts before connect, on connect, on disconnect. Conditional connections (before-connect script allows or denies, or supplies credentials).
- AppleScript dictionary (macOS): `connect`, `disconnect`, `connectall`, `disconnectall`, connection properties (`name`, `state`, `IPv4Address`, `IPv6Address`, `serverAddress`, `networkInterface`, `timeConnected`).
- PowerShell/COM (Windows), D-Bus (Linux). Command line control on all platforms.
- Custom menu items.

**Enterprise**
- Preconfigured deployment (connections overwrite/append/prompt, preferences, menu items).
- Managed preferences (MDM profiles, GPO, Jamf, Munki).
- Standard users connect without admin rights.

## 2. Formats

- Connection storage: `<data dir>/connections/<id>/config.conf` plus referenced files.
- GUI settings live in `config.conf` as `#fluidity <key> <args>` lines. OpenVPN treats them as comments.
- Bundle: `<Name>.fvpn` folder with `config.conf` and files. Archive: `<Name>.fvpnz` (tar.gz of the bundle).
- Custom menu item: `<Name>.fluiditymenuitem` folder with a script and `icon.png`.

## 3. Architecture

```
                 +------------------+       +-------------------+
 user session    |  Native UI       |       |  fluidity (CLI)   |
                 |  macOS: Swift    |       |  Rust             |
                 |  Linux: GTK4/Rust|       +---------+---------+
                 |  Windows: GTK4   |                 |
                 +--------+---------+                 |
                          |  JSON-RPC over local IPC  |
                          v  (Unix socket / named pipe, peer-credential auth)
 privileged      +-----------------------------------------------+
                 |  fluidityd (Rust, root / SYSTEM service)      |
                 |  - session manager, state machine per conn    |
                 |  - OpenVPN management client                  |
                 |  - platform net: DNS, routes, proxy, IPv6 blk |
                 |  - lyrebird, script runner, watchers          |
                 +--------------------+--------------------------+
                                      | spawns, drops to _fluidity user
                                      v
                          openvpn 2.7  +  lyrebird
```

OpenVPN runs with `--management <socket> unix --management-hold --management-query-passwords --management-query-remote --management-query-proxy --auth-retry interact --script-security 1 --setenv IV_GUI_VER Fluidity_<version>`. It runs no scripts. The daemon applies DNS, proxies, and IPv6 blocking itself, using state and pushed options from the management interface.

### Rust workspace

| Crate | Purpose |
|---|---|
| `fluidity-config` | OpenVPN config parser and writer, `#fluidity` meta lines, inline blocks, unsafe-command filter, import/export. |
| `fluidity-mgmt` | Async OpenVPN management interface client (tokio). |
| `fluidity-core` | Connection model, folders, preferences, state machine, reconnect policy, credentials API. |
| `fluidity-ipc` | IPC protocol types and transport. |
| `fluidity-net` | Platform network backends (trait + macOS/Linux/Windows impls). |
| `fluidityd` | Privileged daemon. |
| `fluidity-cli` | `fluidity connect|disconnect|status|import|export|list`. |
| `fluidity-ffi` | C ABI for the Swift UI. |

### Platform backends

| Concern | macOS | Linux | Windows |
|---|---|---|---|
| Tunnel | utun | tun/tap | ovpn-dco-win / wintun / tap-windows6 |
| DNS full/split | SCDynamicStore `State:/Network/Service/<id>/DNS` with `SupplementalMatchDomains` | systemd-resolved D-Bus, resolvconf fallback | NRPT rules + interface DNS |
| IPv6 block | pf anchor | nftables table | WFP filters |
| Proxy on VPN iface | SCDynamicStore `Proxies` | desktop settings (best effort) | WinINet per-connection |
| Credentials | Keychain | Secret Service | Credential Manager |
| Service | launchd daemon | systemd unit + polkit | Windows service |
| Network change | SCDynamicStore / NWPathMonitor | netlink | NotifyIpInterfaceChange |
| Sleep/wake | IOKit power notifications | logind PrepareForSleep | WM_POWERBROADCAST |
| Automation | AppleScript + CLI | D-Bus + CLI | PowerShell + CLI |

## 4. Phases

Each phase ends with tests passing and a commit.

1. **Foundation.** Workspace, license, CI. `fluidity-config` with round-trip tests.
2. **Management client.** `fluidity-mgmt` with a mock OpenVPN server for tests.
3. **Daemon MVP (macOS).** `fluidityd` launches OpenVPN as `_fluidity`, handles hold and auth, reports state. CLI connects a real profile end to end.
4. **macOS network.** Full and split DNS, WINS, routes options, IPv6 block, interface proxies, DNS restore on crash.
5. **macOS UI.** Menu bar app, details window, preferences, connection editor, import, notifications, welcome.
6. **Auth breadth.** Keychain storage, static/dynamic challenge, PKCS#12, PKCS#11, Keychain identities, FIDO, web auth.
7. **Transport and behaviour.** Proxies to server, obfuscation, reconnect on wake/network, idle disconnect, Time Machine, server fallback, OpenVPN version selection.
8. **Automation.** Scripts, conditional connections, AppleScript, menu items, CLI parity.
9. **Enterprise.** Preconfigured deployment, managed preferences, `.fvpnz` export, `.pkg` installer.
10. **Linux.** systemd service, network backend, GTK4 UI with tray, deb/rpm/Flatpak.
11. **Windows.** Service, network backend, GTK4 app with a tray, MSI.
12. **Updates and release.** Sparkle 2 (macOS), WinSparkle (Windows), distro packages (Linux). Reproducible OpenVPN and lyrebird builds.

### Status

- Phases 1 to 9 are done on macOS. FIDO2 is open.
- Phase 10 is done except Flatpak: GTK4 and libadwaita app with an SNI tray, systemd service, deb and rpm packages, a GUI e2e test in Docker. Keychain identities are macOS only.
- Phase 11 is code complete and untested on Windows: service with an ACL-protected named pipe, TCP management with a password, NRPT DNS, IPv6 firewall block, the GTK app with a Win32 tray, a WiX MSI and a CI job. Interface proxies and certificate store identities are not on Windows.
- Phase 12: the GitHub release updater and reproducible OpenVPN and lyrebird builds are done.

## 5. Third-party components (all FOSS)

| Component | License |
|---|---|
| OpenVPN 2.7 | GPL-2.0 |
| lyrebird | BSD-3-Clause |
| libfido2 | BSD-2-Clause |
| Sparkle 2 / WinSparkle | MIT |
| OpenSC (PKCS#11, optional) | LGPL-2.1 |
