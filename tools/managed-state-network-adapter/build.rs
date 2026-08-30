use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_BUILDER_APPLICATION_RS");
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_BUILDER_NATIVE_SOURCES");
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_BUILDER_NATIVE_INCLUDES");

    let (application, configured) = match env::var_os("JAMSCRIPT_BUILDER_APPLICATION_RS") {
        Some(path) => (PathBuf::from(path), true),
        None => {
            let path = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"))
                .join("placeholder_builder_application.rs");
            fs::write(
                &path,
                "pub const JAMSCRIPT_RUNTIME_REFINE_INPUT_VERSION: u8 = 1;\npub struct GeneratedApplication;\nimpl service_runtime_core::ServiceApplication for GeneratedApplication { type Error = service_runtime_core::StateAccessError; fn execute(&self, _: &mut service_runtime_core::ExecutionContext<'_>, _: &[u8]) -> Result<(), Self::Error> { Err(service_runtime_core::StateAccessError::Backend) } }\n",
            )
            .expect("write placeholder builder application");
            (path, false)
        }
    };
    println!(
        "cargo:rustc-env=JAMSCRIPT_BUILDER_ARTIFACT_CONFIGURED={}",
        u8::from(configured)
    );
    println!("cargo:rerun-if-changed={}", application.display());
    println!(
        "cargo:rustc-env=JAMSCRIPT_BUILDER_APPLICATION_RS={}",
        application.display()
    );

    let sources = env::var_os("JAMSCRIPT_BUILDER_NATIVE_SOURCES")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    if sources.is_empty() {
        return;
    }
    let mut build = cc::Build::new();
    for source in sources {
        println!("cargo:rerun-if-changed={}", source.display());
        build.file(source);
    }
    if let Some(includes) = env::var_os("JAMSCRIPT_BUILDER_NATIVE_INCLUDES") {
        for include in env::split_paths(&includes) {
            build.include(include);
        }
    }
    build.compile("jamscript_builder_native");
}
