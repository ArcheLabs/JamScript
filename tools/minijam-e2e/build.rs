use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(jamscript_e2e_native_host)");
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_E2E_BUILDER_APPLICATION_RS");
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_E2E_SCRIPTC_ARCHIVE");
    println!("cargo:rerun-if-env-changed=JAMSCRIPT_E2E_NATIVE_SOURCE");

    let (application, configured) = match env::var_os("JAMSCRIPT_E2E_BUILDER_APPLICATION_RS") {
        Some(path) => (PathBuf::from(path), true),
        None => {
            let path = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"))
                .join("placeholder_builder_application.rs");
            fs::write(
                &path,
                "pub const JAMSCRIPT_RUNTIME_REFINE_INPUT_VERSION: u8 = 1;\npub struct GeneratedApplication;\nimpl service_runtime_core::ServiceApplication for GeneratedApplication { type Error = service_runtime_core::StateAccessError; fn execute(&self, _: &mut service_runtime_core::ExecutionContext<'_>, _: &[u8]) -> Result<(), Self::Error> { Err(service_runtime_core::StateAccessError::Backend) } }\n",
            )
            .expect("write placeholder application");
            (path, false)
        }
    };
    println!("cargo:rerun-if-changed={}", application.display());
    println!(
        "cargo:rustc-env=JAMSCRIPT_E2E_BUILDER_CONFIGURED={}",
        u8::from(configured)
    );
    println!(
        "cargo:rustc-env=JAMSCRIPT_E2E_BUILDER_APPLICATION_RS={}",
        application.display()
    );

    if let Some(archive) = env::var_os("JAMSCRIPT_E2E_SCRIPTC_ARCHIVE") {
        let archive = PathBuf::from(archive);
        println!("cargo:rerun-if-changed={}", archive.display());
        let native_source = env::var_os("JAMSCRIPT_E2E_NATIVE_SOURCE").map(PathBuf::from);
        if native_source.is_some() {
            println!("cargo:rustc-cfg=jamscript_e2e_native_host");
        }
        let host_archive = build_host_scriptc_archive(&archive, native_source.as_ref());
        println!("cargo:rerun-if-changed={}", host_archive.display());
        println!("cargo:rustc-link-arg={}", host_archive.display());
    }
}

fn build_host_scriptc_archive(
    target_archive: &PathBuf,
    native_source: Option<&PathBuf>,
) -> PathBuf {
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let source_dir = target_archive.parent().expect("ScriptC archive directory");
    let workspace =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = workspace.join("toolchains/scriptc/node_modules/@scriptc/runtime/src");
    let include = workspace.join("crates/jamscript-runtime-scriptc/include");
    let generated = source_dir.join("scriptc_service.lib.c");
    let adapter = source_dir.join("scriptc_service_adapter.c");
    println!("cargo:rerun-if-changed={}", generated.display());
    println!("cargo:rerun-if-changed={}", adapter.display());
    let host_shims = output_dir.join("scriptc_host_shims.c");
    fs::write(
        &host_shims,
        r#"#include <math.h>
#include <stddef.h>
#include <string.h>
#include <stdint.h>

#undef isfinite
#undef signbit
int isfinite(double value) { return __builtin_isfinite(value); }
int signbit(double value) { return __builtin_signbit(value); }

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

static const uint8_t *jamscript_e2e_extrinsic;
static size_t jamscript_e2e_extrinsic_size;
static size_t jamscript_e2e_extrinsic_count;

void jamscript_e2e_set_extrinsic(const uint8_t *bytes, size_t size) {
  jamscript_e2e_extrinsic = bytes;
  jamscript_e2e_extrinsic_size = size;
  jamscript_e2e_extrinsic_count = 1;
}

void jamscript_e2e_set_extrinsic_count(size_t count) {
  jamscript_e2e_extrinsic_count = count;
}

uint64_t minijam_host_call(uint32_t call, const uint64_t args[6]) {
  if (call != 1u) return UINT64_MAX;
  if (args[4] >= jamscript_e2e_extrinsic_count) return UINT64_MAX;
  if (args[0] == 0u) return (uint64_t)jamscript_e2e_extrinsic_size;
  if (args[2] < jamscript_e2e_extrinsic_size) return (uint64_t)jamscript_e2e_extrinsic_size;
  memcpy((void *)(uintptr_t)args[0], jamscript_e2e_extrinsic, jamscript_e2e_extrinsic_size);
  return (uint64_t)jamscript_e2e_extrinsic_size;
}
"#,
    )
    .expect("write host ScriptC shims");
    let units = [
        "scr_library.c",
        "scr_number.c",
        "scr_string.c",
        "scr_array.c",
        "scr_bytes.c",
        "scr_closure.c",
        "scr_cycle.c",
        "scr_error.c",
        "scr_exception.c",
        "scr_json.c",
        "scr_object.c",
        "scr_union.c",
    ];
    let clang = env::var_os("JAMSCRIPT_CLANG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/clang"));
    let ar = env::var_os("JAMSCRIPT_LLVM_AR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/llvm-ar"));
    let mut objects = Vec::new();
    let sources = std::iter::once(host_shims)
        .chain(std::iter::once(generated))
        .chain(std::iter::once(adapter))
        .chain(units.iter().map(|unit| runtime.join(unit)))
        .chain(std::iter::once(workspace.join(
            "crates/jamscript-runtime-scriptc/src/scr_lib_cleanup.c",
        )));
    for (index, source) in sources.enumerate() {
        let object = output_dir.join(format!("scriptc_host_{index}.o"));
        let status = Command::new(&clang)
            .args(["-std=c11", "-O0", "-fPIC", "-DSCR_LIB"])
            .arg("-I")
            .arg(&include)
            .arg("-I")
            .arg(&runtime)
            .args(["-c"])
            .arg(&source)
            .args(["-o"])
            .arg(&object)
            .status()
            .unwrap_or_else(|error| panic!("launching host ScriptC compiler: {error}"));
        assert!(
            status.success(),
            "compiling host ScriptC source {}",
            source.display()
        );
        objects.push(object);
    }
    if let Some(native_source) = native_source {
        let native_include = workspace.join("../minijam-client/service-toolchain/sdk/include");
        for (index, source) in [
            native_source.clone(),
            workspace.join("../minijam-client/service-toolchain/sdk/src/minijam.c"),
            workspace.join("../minijam-client/service-toolchain/sdk/src/crypto.c"),
        ]
        .into_iter()
        .enumerate()
        {
            let object = output_dir.join(format!("scriptc_native_host_{index}.o"));
            let status = Command::new(&clang)
                .args(["-std=c11", "-O0", "-fPIC", "-DMINIJAM_HOST_TEST"])
                .arg("-I")
                .arg(&include)
                .arg("-I")
                .arg(&native_include)
                .arg("-c")
                .arg(&source)
                .arg("-o")
                .arg(&object)
                .status()
                .unwrap_or_else(|error| panic!("launching native host compiler: {error}"));
            assert!(
                status.success(),
                "compiling native host source {}",
                source.display()
            );
            objects.push(object);
        }
    }
    let archive = output_dir.join("libscriptc_host.a");
    let status = Command::new(&ar)
        .arg("crs")
        .arg(&archive)
        .args(&objects)
        .status()
        .unwrap_or_else(|error| panic!("launching host ScriptC archiver: {error}"));
    assert!(status.success(), "archiving host ScriptC runtime");
    archive
}
