use std::process::Command;

#[cfg_attr(windows, allow(dead_code))]
pub fn parse_nmcli(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|l| l.strip_prefix("yes:"))
        .map(|s| s.replace("\\:", ":"))
        .filter(|s| !s.is_empty())
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_netsh(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "SSID").then(|| value.trim().to_string())
        })
        .filter(|s| !s.is_empty())
}

#[cfg(windows)]
pub fn current_ssid() -> Option<String> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    parse_netsh(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(windows))]
pub fn current_ssid() -> Option<String> {
    let output = Command::new("nmcli")
        .args(["-t", "-f", "active,ssid", "device", "wifi", "list", "--rescan", "no"])
        .output()
        .ok()?;
    parse_nmcli(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_network_names() {
        assert_eq!(
            parse_nmcli("no:Other\nyes:Cafe\\: Guest\n").as_deref(),
            Some("Cafe: Guest")
        );
        assert_eq!(parse_nmcli("no:Other\n"), None);
        let netsh = "    Name                   : Wi-Fi\r\n    BSSID                  : aa:bb\r\n    SSID                   : Home Net\r\n";
        assert_eq!(parse_netsh(netsh).as_deref(), Some("Home Net"));
        assert_eq!(parse_netsh("    State : disconnected\r\n"), None);
    }
}
