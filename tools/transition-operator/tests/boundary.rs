use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[test]
fn offline_tool_has_no_runtime_dependency() {
    let root = crate_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();

    assert!(manifest.contains("[workspace]"));
    assert!(!manifest.contains("librespot-playback"));
    assert!(!manifest.contains("librespot-connect"));

    let source_root = root.join("src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                assert!(!source.contains("librespot_playback"));
                assert!(!source.contains("librespot_connect"));
            }
        }
    }
}
