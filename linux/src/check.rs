use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub ip: Option<String>,
    pub location: Option<String>,
    pub network: Option<String>,
    pub latency: Option<f64>,
}

#[derive(Deserialize)]
struct Info {
    ip: Option<String>,
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
    org: Option<String>,
}

fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

pub fn run(server: &str) -> Report {
    let server = server.to_string();
    let ping = std::thread::spawn(move || latency(&server));
    let mut report = command("curl")
        .args([
            "-s",
            "-m",
            "10",
            "-H",
            "Accept: application/json",
            "https://ipinfo.io/json",
        ])
        .output()
        .ok()
        .and_then(|o| parse_info(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    report.latency = ping.join().ok().flatten();
    report
}

fn latency(host: &str) -> Option<f64> {
    if host.is_empty() {
        return None;
    }
    let args: &[&str] = if cfg!(windows) {
        &["-n", "3", "-w", "2000"]
    } else {
        &["-c", "3", "-i", "0.2", "-W", "2", "-q"]
    };
    let output = command("ping").args(args).arg(host).output().ok()?;
    parse_ping(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_info(json: &str) -> Option<Report> {
    let info: Info = serde_json::from_str(json).ok()?;
    let place: Vec<String> = [info.city, info.region, info.country]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    Some(Report {
        ip: info.ip,
        location: (!place.is_empty()).then(|| place.join(", ")),
        network: info.org,
        latency: None,
    })
}

pub fn parse_ping(output: &str) -> Option<f64> {
    for line in output.lines() {
        if let Some(values) = line
            .split_once("min/avg/max")
            .and_then(|(_, rest)| rest.split_once('='))
        {
            return values.1.trim().split('/').nth(1)?.trim().parse().ok();
        }
        if let Some((_, average)) = line.rsplit_once("Average = ") {
            return average.trim().trim_end_matches("ms").parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ping_and_info() {
        let linux = "--- 10.0.0.1 ping statistics ---\n3 packets transmitted, 3 received\nrtt min/avg/max/mdev = 0.045/12.5/20.1/0.01 ms\n";
        assert_eq!(parse_ping(linux), Some(12.5));
        let windows = "    Minimum = 10ms, Maximum = 14ms, Average = 12ms\n";
        assert_eq!(parse_ping(windows), Some(12.0));
        assert_eq!(parse_ping("100% packet loss"), None);
        let report =
            parse_info(r#"{"ip":"1.2.3.4","city":"Sydney","region":"","country":"AU","org":"AS1 Example"}"#).unwrap();
        assert_eq!(report.ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(report.location.as_deref(), Some("Sydney, AU"));
        assert_eq!(report.network.as_deref(), Some("AS1 Example"));
    }
}
