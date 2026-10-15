use std::fs;
use std::io::Write;
use std::path::Path;

use fluidity_config::{Bundle, Error};

const OVPN: &str = "\
#fluidity name \"Lab\"
#fluidity connectedscript /scripts/up.scpt
client
remote vpn.example.com 1194 udp
ca keys/ca.crt
cert client.crt
key client.key
tls-auth other/ca.crt 1
<tls-crypt>
INLINE
</tls-crypt>
";

fn setup(root: &Path) -> std::path::PathBuf {
    fs::create_dir_all(root.join("keys")).unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(root.join("keys/ca.crt"), "CA").unwrap();
    fs::write(root.join("other/ca.crt"), "TA").unwrap();
    fs::write(root.join("client.crt"), "CERT").unwrap();
    fs::write(root.join("client.key"), "KEY").unwrap();
    fs::write(root.join("scripts/up.scpt"), "SCRIPT").unwrap();
    let script = root.join("scripts/up.scpt").to_string_lossy().replace('\\', "/");
    let text = OVPN.replace("/scripts/up.scpt", &script);
    let path = root.join("lab.ovpn");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn imports_ovpn_and_collects_files() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = Bundle::import(&setup(tmp.path())).unwrap();

    assert_eq!(bundle.name(), Some("Lab"));
    assert_eq!(bundle.document.directive("ca").unwrap()[0], "ca.crt");
    assert_eq!(bundle.document.directive("tls-auth").unwrap(), ["ca-2.crt", "1"]);
    assert_eq!(bundle.document.meta("connectedscript"), Some("up.scpt"));
    assert_eq!(bundle.document.inline("tls-crypt"), Some("INLINE\n"));
    let names: Vec<_> = bundle.files.keys().cloned().collect();
    assert_eq!(names, ["ca-2.crt", "ca.crt", "client.crt", "client.key", "up.scpt"]);
    assert_eq!(bundle.files["ca-2.crt"], b"TA");
}

#[test]
fn archive_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = Bundle::import(&setup(tmp.path())).unwrap();
    let archive = tmp.path().join("Lab.fvpnz");
    fs::write(&archive, bundle.to_archive("Lab").unwrap()).unwrap();

    let again = Bundle::import(&archive).unwrap();
    assert_eq!(again, bundle);

    let out = tmp.path().join("out.fvpn");
    again.write_dir(&out).unwrap();
    assert_eq!(Bundle::import(&out).unwrap(), bundle);
}

#[test]
fn imports_zip_and_rejects_traversal() {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("Office.fvpn/config.conf", opts).unwrap();
        zip.write_all(b"client\nca ca.crt\n").unwrap();
        zip.start_file("Office.fvpn/ca.crt", opts).unwrap();
        zip.write_all(b"CA").unwrap();
        zip.finish().unwrap();
    }
    let bundle = Bundle::from_archive(buf.get_ref()).unwrap();
    assert_eq!(bundle.files["ca.crt"], b"CA");

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("x.fvpn/config.conf", opts).unwrap();
        zip.write_all(b"client\nca ../../etc/passwd\n").unwrap();
        zip.finish().unwrap();
    }
    assert!(matches!(Bundle::from_archive(buf.get_ref()), Err(Error::UnsafePath(_))));
}

#[test]
fn imports_tunnelblick_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let res = tmp.path().join("Home.tblk/Contents/Resources");
    fs::create_dir_all(&res).unwrap();
    fs::write(res.join("home.ovpn"), "client\nca ca.crt\n").unwrap();
    fs::write(res.join("ca.crt"), "CA").unwrap();
    let bundle = Bundle::import(&tmp.path().join("Home.tblk")).unwrap();
    assert_eq!(bundle.files["ca.crt"], b"CA");
}

#[test]
fn reports_missing_reference() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.ovpn");
    fs::write(&path, "client\nca nope.crt\n").unwrap();
    assert!(matches!(Bundle::import(&path), Err(Error::Io { .. })));
}
