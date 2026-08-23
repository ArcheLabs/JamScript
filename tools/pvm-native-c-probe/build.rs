fn main() {
    let value = std::env::var("SERVICE_BUILD_POLKAVM_NATIVE_ARCHIVES").unwrap_or_default();
    for entry in value.lines() {
        let Some((name, path)) = entry.split_once('=') else { continue };
        let path = std::path::Path::new(path);
        if let Some(parent) = path.parent() {
            println!("cargo:rustc-link-search=native={}", parent.display());
        }
        println!("cargo:rustc-link-lib=static={name}");
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
