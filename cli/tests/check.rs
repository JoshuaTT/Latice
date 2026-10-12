//! `lattice check` end to end: run the real binary on fixture modules.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn default_ruleset() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml")
}

fn lattice_check(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .arg("check")
        .arg("--ruleset")
        .arg(default_ruleset())
        .args(args)
        .output()
        .expect("run lattice")
}

fn s(p: &Path) -> &std::ffi::OsStr {
    p.as_os_str()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn valid_module_passes() {
    let valid = fixture("valid.py");
    let out = lattice_check(&[s(&valid)]);
    assert!(out.status.success(), "{}", stdout(&out));
    assert!(stdout(&out).contains("valid.py: ok ("), "{}", stdout(&out));
}

#[test]
fn banned_construct_fails_with_line_and_code() {
    let banned = fixture("banned.py");
    let out = lattice_check(&[s(&banned)]);
    assert!(!out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains("banned.py:6:5: error E_BANNED_NAME:"),
        "{text}"
    );
    assert!(text.contains("banned.py: 1 errors"), "{text}");
}

#[test]
fn library_over_the_ruleset_quota_fails() {
    // The limit must come from the ruleset, so tighten a copy of it.
    let dir = tempfile::tempdir().unwrap();
    let text = std::fs::read_to_string(default_ruleset()).unwrap();
    let tight = text.replace("team_complexity_nodes = 2400", "team_complexity_nodes = 1");
    assert_ne!(
        text, tight,
        "default ruleset quota line changed; update this test"
    );
    let ruleset = dir.path().join("tight.toml");
    std::fs::write(&ruleset, tight).unwrap();

    let (a, b) = (fixture("valid.py"), fixture("valid_two.py"));
    let out = Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(["check", "--library", "--ruleset"])
        .arg(&ruleset)
        .args([&a, &b])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stdout(&out).contains("/ 1 nodes"), "{}", stdout(&out));
}

#[test]
fn library_within_quota_passes_and_quota_flag_overrides() {
    let (a, b) = (fixture("valid.py"), fixture("valid_two.py"));
    let ok = lattice_check(&[s("--library".as_ref()), s(&a), s(&b)]);
    assert!(ok.status.success(), "{}", stdout(&ok));
    assert!(stdout(&ok).contains("/ 2400 nodes"), "{}", stdout(&ok));

    let low = lattice_check(&[
        s("--library".as_ref()),
        s("--quota".as_ref()),
        s("1".as_ref()),
        s(&a),
        s(&b),
    ]);
    assert!(!low.status.success());
    assert!(stdout(&low).contains("/ 1 nodes"), "{}", stdout(&low));
}

#[test]
fn quota_without_library_is_rejected() {
    let a = fixture("valid.py");
    let out = lattice_check(&[s("--quota".as_ref()), s("5".as_ref()), s(&a)]);
    assert!(!out.status.success());
}

#[test]
fn no_quota_check_without_library() {
    let (a, b) = (fixture("valid.py"), fixture("valid_two.py"));
    let out = lattice_check(&[s(&a), s(&b)]);
    assert!(out.status.success());
    assert!(!stdout(&out).contains("library:"));
}

#[test]
fn json_output_parses_with_all_fields_in_argument_order() {
    let (b, a) = (fixture("banned.py"), fixture("valid_two.py"));
    let out = lattice_check(&[s("--json".as_ref()), s(&b), s(&a)]);
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    let files = v.as_array().expect("array");
    assert_eq!(files.len(), 2);
    assert!(files[0]["path"].as_str().unwrap().ends_with("banned.py"));
    assert_eq!(files[0]["ok"], false);
    let d = &files[0]["diagnostics"][0];
    assert_eq!(d["severity"], "error");
    assert_eq!(d["code"], "E_BANNED_NAME");
    assert_eq!(d["line"], 6);
    assert!(d["col"].is_u64() && d["message"].is_string());
    assert_eq!(files[1]["ok"], true);
    assert!(files[1]["node_count"].as_u64().unwrap() > 0);
    let iface = &files[1]["interface"];
    for (key, want) in [
        ("init", true),
        ("tick", true),
        ("on_hit", true),
        ("on_hacked", false),
        ("on_radio", false),
    ] {
        assert_eq!(iface[key], want, "{key}");
    }
}

#[test]
fn missing_file_is_an_error_on_stderr() {
    let missing = fixture("does_not_exist.py");
    let out = lattice_check(&[s(&missing)]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("does_not_exist.py"), "{err}");
}
