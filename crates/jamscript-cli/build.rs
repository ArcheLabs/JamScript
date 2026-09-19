use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_RELEASE_VERSION");
    let version = env::var("JAMSCRIPT_RELEASE_VERSION").unwrap_or_else(|_| "0.1.0-dev".into());
    println!("cargo:rustc-env=JAMSCRIPT_RELEASE_VERSION={version}");
}
