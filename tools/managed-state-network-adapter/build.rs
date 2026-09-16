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
    let sources = sources
        .into_iter()
        .filter(|source| !source.as_os_str().is_empty())
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return;
    }
    let host_shims =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("scriptc_host_shims.c");
    fs::write(
        &host_shims,
        r#"#include <math.h>
#include <stdint.h>

static uint32_t jamscript_to_uint32(double value) {
  if (!isfinite(value)) return 0;
  double truncated = fmod(trunc(value), 4294967296.0);
  if (truncated < 0.0) truncated += 4294967296.0;
  return (uint32_t)truncated;
}

static double jamscript_bits_as_int32(uint32_t value) {
  return value >= UINT32_C(0x80000000)
             ? (double)(int32_t)(value - UINT32_C(0x80000000)) + (double)INT32_MIN
             : (double)value;
}

double scr_bit_and(double a, double b) { return jamscript_bits_as_int32(jamscript_to_uint32(a) & jamscript_to_uint32(b)); }
double scr_bit_or(double a, double b) { return jamscript_bits_as_int32(jamscript_to_uint32(a) | jamscript_to_uint32(b)); }
double scr_bit_xor(double a, double b) { return jamscript_bits_as_int32(jamscript_to_uint32(a) ^ jamscript_to_uint32(b)); }
double scr_bit_shl(double a, double b) { return jamscript_bits_as_int32(jamscript_to_uint32(a) << (jamscript_to_uint32(b) & 31u)); }
double scr_bit_shr(double a, double b) {
  uint32_t value = jamscript_to_uint32(a), shift = jamscript_to_uint32(b) & 31u;
  uint32_t result = value >> shift;
  if ((value & UINT32_C(0x80000000)) != 0 && shift != 0) result |= ~(UINT32_C(0xffffffff) >> shift);
  return jamscript_bits_as_int32(result);
}
double scr_bit_ushr(double a, double b) { return (double)(jamscript_to_uint32(a) >> (jamscript_to_uint32(b) & 31u)); }
double scr_bit_not(double value) { return jamscript_bits_as_int32(~jamscript_to_uint32(value)); }
"#,
    )
    .expect("write host ScriptC shims");
    println!("cargo:rerun-if-changed={}", host_shims.display());
    let mut native_sources = vec![host_shims];
    native_sources.extend(sources);
    let mut build = cc::Build::new();
    build.define("SCR_LIB", None);
    for source in native_sources {
        println!("cargo:rerun-if-changed={}", source.display());
        build.file(source);
    }
    if let Some(includes) = env::var_os("JAMSCRIPT_BUILDER_NATIVE_INCLUDES") {
        for include in env::split_paths(&includes) {
            build.include(include);
        }
    }
    build.compile("jamscript_builder_native");

    // The generated service object lives in the same archive as the ScriptC
    // runtime.  It introduces runtime references only after the archive's
    // first scan, so replay the archive inside a linker group as well as the
    // normal cc-rs link directive.
    let archive = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join("libjamscript_builder_native.a");
    println!("cargo:rustc-link-arg-bin=managed-state-network-adapter=-Wl,--start-group");
    println!(
        "cargo:rustc-link-arg-bin=managed-state-network-adapter={}",
        archive.display()
    );
    println!("cargo:rustc-link-arg-bin=managed-state-network-adapter=-Wl,--end-group");
}
