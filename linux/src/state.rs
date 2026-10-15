use fluidity_config::settings::StaticChallengePassword;
use fluidity_core::{Node, Store};

#[derive(Debug, Clone, Default)]
pub struct ConnInfo {
    pub id: String,
    pub name: String,
    pub folder: Option<String>,
    pub credential_scope: String,
    pub auto_reconnect: bool,
    pub connect_on_launch: bool,
    pub before_connect_script: Option<String>,
    pub connected_script: Option<String>,
    pub disconnected_script: Option<String>,
    pub static_challenge_password: Option<StaticChallengePassword>,
    pub static_challenge_acknowledge: Option<String>,
    pub openvpn_version: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct TrafficSample {
    pub time: f64,
    pub in_rate: f64,
    pub out_rate: f64,
}

pub const HISTORY: usize = 300;

pub fn load(store: &Store) -> (Vec<Node>, Vec<ConnInfo>) {
    let tree = store.tree();
    let list = store
        .connections()
        .into_iter()
        .map(|c| {
            let folder = c.folder.as_deref().and_then(|f| store.folder(f));
            let scope = folder
                .as_ref()
                .filter(|f| f.shared_credentials)
                .map(|f| f.id.clone())
                .unwrap_or_else(|| c.id.clone());
            let script = |own: &Option<String>, shared: Option<&Option<String>>| {
                own.as_ref()
                    .map(|name| c.dir.join(name).to_string_lossy().into_owned())
                    .or_else(|| shared.cloned().flatten())
            };
            let s = &c.settings;
            ConnInfo {
                credential_scope: scope,
                auto_reconnect: s.auto_reconnect,
                connect_on_launch: s.connect_on_launch,
                before_connect_script: script(
                    &s.scripts.before_connect,
                    folder.as_ref().map(|f| &f.before_connect_script),
                ),
                connected_script: script(&s.scripts.connected, folder.as_ref().map(|f| &f.connected_script)),
                disconnected_script: script(&s.scripts.disconnected, folder.as_ref().map(|f| &f.disconnected_script)),
                static_challenge_password: s.static_challenge_password.clone(),
                static_challenge_acknowledge: s.static_challenge_acknowledge.clone(),
                openvpn_version: s.openvpn_version.clone(),
                id: c.id,
                name: c.name,
                folder: c.folder,
            }
        })
        .collect();
    (tree, list)
}

pub fn bytes(value: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = value.max(0.0);
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", v as u64)
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

pub fn usage_days(
    samples: &[fluidity_ipc::UsageSample],
    count: usize,
    today: i64,
    day_of: impl Fn(u64) -> i64,
) -> Vec<(u64, u64)> {
    let mut days = vec![(0, 0); count];
    for s in samples {
        let back = today - day_of(s.hour);
        if (0..count as i64).contains(&back) {
            let day = &mut days[count - 1 - back as usize];
            day.0 += s.bytes_in;
            day.1 += s.bytes_out;
        }
    }
    days
}

pub fn usage_total(days: &[(u64, u64)]) -> (u64, u64) {
    days.iter().fold((0, 0), |(a, b), (i, o)| (a + i, b + o))
}

pub fn ticks(peak: f64) -> Vec<f64> {
    if peak <= 0.0 {
        return vec![0.0];
    }
    let mut unit = 1.0;
    while peak / unit >= 1024.0 {
        unit *= 1024.0;
    }
    let step = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0]
        .iter()
        .map(|m| m * unit)
        .find(|step| peak / step <= 4.0)
        .unwrap_or(1024.0 * unit);
    (0..).map(|k| k as f64 * step).take_while(|v| *v <= peak).collect()
}

pub fn duration(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

pub fn secret_field(realm: &str) -> String {
    realm.to_lowercase().replace(' ', "-")
}

pub fn walk_folders(nodes: &[Node], out: &mut Vec<(String, String)>) {
    for node in nodes {
        if let Node::Folder(f) = node {
            out.push((f.id.clone(), f.name.clone()));
            walk_folders(&f.children, out);
        }
    }
}

pub fn detach(nodes: &mut Vec<Node>, id: &str) -> bool {
    if let Some(i) = nodes.iter().position(|n| matches!(n, Node::Connection(c) if c == id)) {
        nodes.remove(i);
        return true;
    }
    nodes.iter_mut().any(|n| match n {
        Node::Folder(f) => detach(&mut f.children, id),
        Node::Connection(_) => false,
    })
}

pub fn move_to(nodes: &mut Vec<Node>, id: &str, folder: Option<&str>) {
    detach(nodes, id);
    fn insert(nodes: &mut [Node], id: &str, folder: &str) -> bool {
        nodes.iter_mut().any(|n| match n {
            Node::Folder(f) if f.id == folder => {
                f.children.push(Node::Connection(id.to_string()));
                true
            }
            Node::Folder(f) => insert(&mut f.children, id, folder),
            Node::Connection(_) => false,
        })
    }
    if !folder.is_some_and(|f| insert(nodes, id, f)) {
        nodes.push(Node::Connection(id.to_string()));
    }
}

pub fn shift(nodes: &mut [Node], id: &str, delta: isize) -> bool {
    let position = nodes.iter().position(|n| match n {
        Node::Connection(c) => c == id,
        Node::Folder(f) => f.id == id,
    });
    if let Some(i) = position {
        let j = i as isize + delta;
        if j >= 0 && (j as usize) < nodes.len() {
            nodes.swap(i, j as usize);
        }
        return true;
    }
    nodes.iter_mut().any(|n| match n {
        Node::Folder(f) => shift(&mut f.children, id, delta),
        Node::Connection(_) => false,
    })
}

pub fn remove_folder(nodes: &mut Vec<Node>, id: &str) -> bool {
    if let Some(i) = nodes.iter().position(|n| matches!(n, Node::Folder(f) if f.id == id)) {
        let Node::Folder(f) = nodes.remove(i) else {
            unreachable!()
        };
        for (k, child) in f.children.into_iter().enumerate() {
            nodes.insert(i + k, child);
        }
        return true;
    }
    nodes.iter_mut().any(|n| match n {
        Node::Folder(f) => remove_folder(&mut f.children, id),
        Node::Connection(_) => false,
    })
}

pub fn folder_mut<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut fluidity_core::Folder> {
    for n in nodes.iter_mut() {
        if let Node::Folder(f) = n {
            if f.id == id {
                return Some(f);
            }
            if let Some(found) = folder_mut(&mut f.children, id) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluidity_core::Folder;
    use fluidity_ipc::UsageSample;

    #[test]
    fn groups_usage_by_day() {
        let sample = |hour, bytes_in| UsageSample {
            hour,
            bytes_in,
            bytes_out: 1,
        };
        let samples = [
            sample(10 * 86400, 5),
            sample(10 * 86400 + 3600, 7),
            sample(8 * 86400, 2),
            sample(1, 9),
        ];
        let days = usage_days(&samples, 3, 10, |t| (t / 86400) as i64);
        assert_eq!(days, vec![(2, 1), (0, 0), (12, 2)]);
        assert_eq!(usage_total(&days), (14, 3));
        assert_eq!(ticks(3.4 * 1024.0 * 1024.0 * 1024.0).len(), 4);
        assert_eq!(ticks(900.0), vec![0.0, 500.0]);
        assert_eq!(ticks(0.0), vec![0.0]);
    }

    fn folder(id: &str, children: Vec<Node>) -> Node {
        Node::Folder(Folder {
            id: id.into(),
            name: id.into(),
            children,
            shared_credentials: false,
            before_connect_script: None,
            connected_script: None,
            disconnected_script: None,
        })
    }

    #[test]
    fn tree_edits() {
        let c = |id: &str| Node::Connection(id.into());
        let mut tree = vec![c("a"), folder("f", vec![c("b")]), c("d")];
        move_to(&mut tree, "a", Some("f"));
        assert_eq!(tree.len(), 2);
        let Node::Folder(f) = &tree[0] else { panic!() };
        assert_eq!(f.children, vec![c("b"), c("a")]);
        shift(&mut tree, "a", -1);
        let Node::Folder(f) = &tree[0] else { panic!() };
        assert_eq!(f.children, vec![c("a"), c("b")]);
        shift(&mut tree, "d", -1);
        assert_eq!(tree[0], c("d"));
        remove_folder(&mut tree, "f");
        assert_eq!(tree, vec![c("d"), c("a"), c("b")]);
        move_to(&mut tree, "b", Some("missing"));
        assert_eq!(tree.last(), Some(&c("b")));
    }

    #[test]
    fn formats() {
        assert_eq!(bytes(10.0), "10 B");
        assert_eq!(bytes(2048.0), "2.0 KB");
        assert_eq!(duration(65), "1:05");
        assert_eq!(duration(3661), "1:01:01");
        assert_eq!(secret_field("Private Key"), "private-key");
    }
}
