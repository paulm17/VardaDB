#[test]
fn standalone_cargo_manifest_keeps_the_runtime_surface_small() {
    let manifest = include_str!("../Cargo.toml");

    for required in [
        "restate-core",
        "restate-sqlite-store",
        "restate-storage-api",
        "restate-worker",
    ] {
        assert!(
            manifest.contains(required),
            "standalone manifest unexpectedly misses {required}"
        );
    }
}

#[test]
fn standalone_dependency_tree_contains_the_expected_local_runtime_crates() {
    let output = std::process::Command::new("cargo")
        .args(["tree", "-p", "restate-standalone", "--prefix", "none"])
        .output()
        .expect("run cargo tree for standalone");

    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("cargo tree stdout is utf8");
    for required in [
        "restate-core",
        "restate-sqlite-store",
        "restate-storage-api",
        "restate-worker",
    ] {
        assert!(
            stdout.lines().any(|line| line.contains(required)),
            "standalone dependency tree unexpectedly misses {required}:\n{stdout}"
        );
    }
}

#[test]
fn standalone_sources_keep_the_local_runtime_entrypoints() {
    let combined = [
        include_str!("../src/config.rs"),
        include_str!("../src/metadata.rs"),
        include_str!("../src/standalone.rs"),
        include_str!("../src/worker.rs"),
    ]
    .join("\n");

    for required in ["Standalone", "sqlite", "node_name"] {
        assert!(
            combined.contains(required),
            "standalone source unexpectedly misses {required}"
        );
    }
}

#[test]
fn repo_surface_keeps_required_runtime_assets() {
    for present_path in [
        "standalone/Cargo.toml",
        "standalone/src/main.rs",
        "crates/core/Cargo.toml",
        "crates/sqlite-store/Cargo.toml",
    ] {
        assert!(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(present_path)
                .exists(),
            "required runtime asset is missing at {present_path}"
        );
    }
}

#[test]
fn current_readmes_advertise_the_supported_runtime_and_cli() {
    for readme in [
        include_str!("../../README.md"),
        include_str!("../README.md"),
    ] {
        assert!(readme.contains("restate-standalone"));
    }
}
