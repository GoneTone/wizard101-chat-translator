//! 版本號：單一來源是 Cargo.toml，放版 workflow 收得下的 tag 這裡也必須收得下。

use std::path::{Path, PathBuf};

use regex::Regex;

/// 允許預發布後綴（0.2.0-rc.1）：放版 workflow 收這種 tag。
fn semver() -> Regex {
    Regex::new(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$").unwrap()
}

fn release_workflow() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|dir| dir.join(".github/workflows/release-windows.yml"))
        .find(|path| path.is_file())
        .expect("release-windows.yml not found above the crate")
}

#[test]
fn version_is_semver() {
    let version = env!("CARGO_PKG_VERSION");
    assert!(semver().is_match(version), "{version}");
}

#[test]
fn release_workflow_tags_are_accepted_here() {
    let text = std::fs::read_to_string(release_workflow()).unwrap();
    let pattern = Regex::new(r"-notmatch '(\^v.+\$)'")
        .unwrap()
        .captures(&text)
        .expect("the workflow no longer validates the tag with -notmatch")[1]
        .to_owned();
    let tag_re = Regex::new(&pattern).unwrap();
    for tag in ["v0.1.0", "v0.2.0-rc.1", "v10.20.30-beta.2"] {
        assert!(tag_re.is_match(tag), "the workflow rejects {tag}");
        assert!(
            semver().is_match(&tag[1..]),
            "tag {tag} would be rejected here once written into the version"
        );
    }
}
