use std::env;

fn main() {
    // `gws --version`: the release tag when the release build sets GWS_VERSION, otherwise the
    // Cargo version marked as a dev build. Kept out of Cargo.toml so releases need no
    // version-bump commits.
    println!("cargo:rerun-if-env-changed=GWS_VERSION");
    let version = env::var("GWS_VERSION")
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            format!(
                "{}-dev",
                env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION")
            )
        });
    println!("cargo:rustc-env=GWS_VERSION={version}");
}
