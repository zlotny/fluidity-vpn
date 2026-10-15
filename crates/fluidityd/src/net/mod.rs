use std::path::{Path, PathBuf};

use crate::netconfig::NetConfig;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported;

pub trait Backend: Send + Sync {
    fn apply(&self, id: &str, config: &NetConfig) -> Result<(), String>;
    fn revert(&self, id: &str);
    fn recover(&self);
    fn pause_time_machine(&self) -> Result<(), String> {
        Ok(())
    }
    fn resume_time_machine(&self) -> Result<(), String> {
        Ok(())
    }
}

pub fn backend(state_dir: &Path) -> Box<dyn Backend> {
    #[cfg(target_os = "macos")]
    return Box::new(macos::MacBackend::new(state_dir));
    #[cfg(target_os = "linux")]
    return Box::new(linux::LinuxBackend::new(state_dir));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Box::new(unsupported::Unsupported::new(state_dir));
}

pub(crate) fn state_file(dir: &Path, id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("net-{safe}.json"))
}

pub(crate) fn state_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("net-") && n.ends_with(".json"))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub(crate) fn run_with_input(program: &str, args: &[&str], input: &str) -> Result<String, String> {
    use std::io::Write;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .map_err(|e| format!("{program}: {e}"))?;
    let output = child.wait_with_output().map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
