use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
    pub version: String,
    pub path: PathBuf,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

fn bundled_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(exe) = exe_dir() {
        roots.push(exe.join("openvpn"));
        roots.push(exe.join("../Resources/openvpn"));
        roots.push(exe.join("../lib/fluidity/openvpn"));
    }
    roots.push(PathBuf::from("/Library/Application Support/Fluidity/openvpn"));
    roots.push(PathBuf::from("/usr/lib/fluidity/openvpn"));
    roots
}

#[cfg(windows)]
const SYSTEM_PATHS: &[&str] = &[
    r"C:\Program Files\OpenVPN\bin\openvpn.exe",
    r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
];

#[cfg(not(windows))]
const SYSTEM_PATHS: &[&str] = &[
    "/opt/homebrew/sbin/openvpn",
    "/usr/local/sbin/openvpn",
    "/usr/sbin/openvpn",
    "/usr/bin/openvpn",
    "/sbin/openvpn",
];

fn binary_name() -> &'static str {
    if cfg!(windows) { "openvpn.exe" } else { "openvpn" }
}

pub fn probe_version(path: &Path) -> Option<String> {
    let output = std::process::Command::new(path).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().next()?;
    let version = first.split_whitespace().nth(1)?;
    version.chars().next()?.is_ascii_digit().then(|| version.to_string())
}

pub fn version_key(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

#[cfg(unix)]
pub fn trusted(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    if unsafe { libc::geteuid() } != 0 {
        return true;
    }
    let Ok(real) = path.canonicalize() else {
        return false;
    };
    real.ancestors().all(|p| {
        std::fs::metadata(p)
            .map(|m| m.uid() == 0 && m.mode() & 0o022 == 0)
            .unwrap_or(false)
    })
}

#[cfg(not(unix))]
pub fn trusted(_path: &Path) -> bool {
    true
}

pub fn discover() -> Vec<Binary> {
    let mut found: Vec<Binary> = Vec::new();
    let mut add = |path: PathBuf, check: bool| {
        if !path.is_file() || found.iter().any(|b| b.path == path) || (check && !trusted(&path)) {
            return;
        }
        if let Some(version) = probe_version(&path) {
            found.push(Binary { version, path });
        }
    };
    if let Some(path) = std::env::var_os("FLUIDITY_OPENVPN") {
        add(PathBuf::from(path), false);
    }
    let mut push = |path: PathBuf| add(path, true);
    for root in bundled_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            push(entry.path().join(binary_name()));
        }
    }
    for path in SYSTEM_PATHS {
        push(PathBuf::from(path));
    }
    found.sort_by_key(|b| std::cmp::Reverse(version_key(&b.version)));
    found
}

pub fn select<'a>(binaries: &'a [Binary], requested: Option<&str>) -> Option<&'a Binary> {
    let requested = requested.filter(|r| !r.is_empty() && *r != "latest" && *r != "automatic");
    if let Some(want) = requested {
        let want_key = version_key(want);
        if let Some(b) = binaries.iter().find(|b| version_key(&b.version).starts_with(&want_key)) {
            return Some(b);
        }
    }
    binaries.first()
}

pub enum Management<'a> {
    #[cfg_attr(windows, allow(dead_code))]
    Unix(&'a Path),
    #[cfg_attr(not(windows), allow(dead_code))]
    Tcp { port: u16, password_file: &'a Path },
}

pub struct LaunchSpec<'a> {
    pub management: Management<'a>,
    pub gui_version: &'a str,
    pub compat_mode: Option<&'a str>,
    pub query_proxy: bool,
    pub external_identity: bool,
    pub disable_dns_updown: bool,
}

pub fn arguments(spec: &LaunchSpec, has_verb: bool) -> Vec<String> {
    let mut args: Vec<String> = ["--config", fluidity_config::CONFIG_FILE, "--management"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    match spec.management {
        Management::Unix(path) => {
            args.push(path.to_string_lossy().into_owned());
            args.push("unix".into());
        }
        Management::Tcp { port, password_file } => {
            args.push("127.0.0.1".into());
            args.push(port.to_string());
            args.push(password_file.to_string_lossy().into_owned());
        }
    }
    args.extend(
        [
            "--management-hold",
            "--management-query-passwords",
            "--management-up-down",
            "--auth-retry",
            "interact",
            "--script-security",
            "1",
            "--setenv",
            "IV_GUI_VER",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.push(spec.gui_version.to_string());
    if spec.external_identity {
        args.extend(
            [
                "--management-external-key",
                "pkcs1",
                "pss",
                "--management-external-cert",
                "fluidity",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }
    if spec.disable_dns_updown {
        args.push("--dns-updown".into());
        args.push("disable".into());
    }
    if spec.query_proxy {
        args.push("--management-query-proxy".into());
    }
    if let Some(mode) = spec.compat_mode {
        args.push("--compat-mode".into());
        args.push(mode.into());
    }
    args.push("--log".into());
    args.push(if cfg!(windows) { "NUL" } else { "/dev/null" }.into());
    if !has_verb {
        args.push("--verb".into());
        args.push("3".into());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: &str) -> Binary {
        Binary {
            version: v.into(),
            path: PathBuf::from(format!("/x/{v}/openvpn")),
        }
    }

    #[test]
    fn selects_requested_or_latest() {
        let list = vec![b("2.7.8"), b("2.6.14"), b("2.5.11")];
        assert_eq!(select(&list, None).unwrap().version, "2.7.8");
        assert_eq!(select(&list, Some("2.6")).unwrap().version, "2.6.14");
        assert_eq!(select(&list, Some("2.4")).unwrap().version, "2.7.8");
        assert_eq!(select(&list, Some("latest")).unwrap().version, "2.7.8");
    }

    #[test]
    fn builds_arguments() {
        let sock = PathBuf::from("/run/x/mgmt.sock");
        let args = arguments(
            &LaunchSpec {
                management: Management::Unix(&sock),
                gui_version: "Fluidity_0.1.0",
                compat_mode: Some("2.4.0"),
                query_proxy: false,
                external_identity: false,
                disable_dns_updown: false,
            },
            false,
        );
        let line = args.join(" ");
        assert!(line.starts_with("--config config.conf --management /run/x/mgmt.sock unix --management-hold"));
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        assert!(line.ends_with(&format!("--compat-mode 2.4.0 --log {null} --verb 3")));
        assert!(!line.contains("external"));
    }

    #[test]
    fn requests_external_identity() {
        let sock = PathBuf::from("/run/x/mgmt.sock");
        let args = arguments(
            &LaunchSpec {
                management: Management::Unix(&sock),
                gui_version: "Fluidity_0.1.0",
                compat_mode: None,
                query_proxy: false,
                external_identity: true,
                disable_dns_updown: true,
            },
            true,
        );
        let line = args.join(" ");
        assert!(line.contains("--management-external-key pkcs1 pss --management-external-cert fluidity"));
        assert!(line.contains("--dns-updown disable"));
    }

    #[test]
    fn builds_tcp_management() {
        let pw = PathBuf::from("mgmt.pw");
        let args = arguments(
            &LaunchSpec {
                management: Management::Tcp {
                    port: 50123,
                    password_file: &pw,
                },
                gui_version: "Fluidity_0.1.0",
                compat_mode: None,
                query_proxy: false,
                external_identity: false,
                disable_dns_updown: false,
            },
            true,
        );
        assert!(
            args.join(" ")
                .starts_with("--config config.conf --management 127.0.0.1 50123 mgmt.pw --management-hold")
        );
    }
}
