use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fluidity_ipc::SessionInfo;

use crate::state::ConnInfo;

#[derive(Debug, Default)]
pub struct BeforeConnect {
    pub allow: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub challenge: Option<String>,
}

pub fn environment(
    connection: &ConnInfo,
    session: Option<&SessionInfo>,
    username: Option<&str>,
) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    let values = [
        ("displayName", Some(connection.name.as_str())),
        ("connectionId", Some(connection.id.as_str())),
        ("ipv4Address", session.map(|s| s.local_ipv4.as_str())),
        ("ipv6Address", session.map(|s| s.local_ipv6.as_str())),
        ("serverIPAddress", session.map(|s| s.remote_address.as_str())),
        ("interface", session.map(|s| s.interface.as_str())),
        ("username", username),
    ];
    for (key, value) in values {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            continue;
        };
        env.insert(key.into(), value.into());
        env.insert(format!("FLUIDITY_{}", key.to_uppercase()), value.into());
    }
    env
}

pub fn run(path: &str, env: &HashMap<String, String>, timeout: Duration) -> (i32, String) {
    let script = Path::new(path);
    let mut command = if is_executable(script) {
        Command::new(script)
    } else {
        let mut c = Command::new("/bin/sh");
        c.arg(script);
        c
    };
    if let Some(dir) = script.parent() {
        command.current_dir(dir);
    }
    let child = command
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (-1, e.to_string()),
    };
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_string(&mut out);
        }
        out
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break -1;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break -1,
        }
    };
    (status, reader.join().unwrap_or_default())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn parse_before_connect(status: i32, output: &str) -> BeforeConnect {
    let mut result = BeforeConnect {
        allow: true,
        ..Default::default()
    };
    for line in output.lines().map(str::trim) {
        match line {
            "FluidityNoConnect" => result.allow = false,
            "FluidityConnect" => result.allow = true,
            _ => {}
        }
        if let Some(v) = line.strip_prefix("FluidityUsername:") {
            result.username = Some(v.into());
        }
        if let Some(v) = line.strip_prefix("FluidityPassword:") {
            result.password = Some(v.into());
        }
        if let Some(v) = line.strip_prefix("FluidityChallenge:") {
            result.challenge = Some(v.into());
        }
    }
    if status != 0 && !output.contains("FluidityConnect") {
        result.allow = false;
    }
    result
}

pub fn before_connect(path: &str, connection: &ConnInfo) -> BeforeConnect {
    let (status, output) = run(path, &environment(connection, None, None), Duration::from_secs(120));
    parse_before_connect(status, &output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_before_connect_output() {
        let r = parse_before_connect(
            0,
            "x\nFluidityUsername:bob\nFluidityPassword:pw\nFluidityChallenge:123\n",
        );
        assert!(r.allow);
        assert_eq!(r.username.as_deref(), Some("bob"));
        assert_eq!(r.password.as_deref(), Some("pw"));
        assert_eq!(r.challenge.as_deref(), Some("123"));
        assert!(!parse_before_connect(0, "FluidityNoConnect").allow);
        assert!(!parse_before_connect(1, "").allow);
        assert!(parse_before_connect(1, "FluidityConnect").allow);
    }

    #[test]
    fn runs_scripts_with_environment() {
        let dir = std::env::temp_dir().join(format!("fl-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.sh");
        std::fs::write(&path, "echo \"FluidityUsername:$displayName\"\n").unwrap();
        let conn = ConnInfo {
            id: "abc".into(),
            name: "Lab".into(),
            ..Default::default()
        };
        let r = before_connect(path.to_str().unwrap(), &conn);
        assert_eq!(r.username.as_deref(), Some("Lab"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
