use anyhow::{bail, Context, Result};
use jamscript_codegen_rust::{generate_no_std_rust_with_context, PortableServiceContext};
use jamscript_ir::{abi_for, ServiceIr, NATIVE_ABI_VERSION};
use serde::Serialize;
use service_build_polkavm::{
    GuestBuildArtifacts, NativeArchive, PolkaVmBuildConfig, PolkaVmBuildRequest, PolkaVmBuilder,
};
use service_runtime_core::{
    MANAGED_STATE_LAYOUT_VERSION, MANAGED_STATE_PROTOCOL_VERSION, RECOVERY_FORMAT_VERSION,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::tempdir;

#[derive(Clone, Debug, Serialize)]
pub struct BuildMetadata {
    #[serde(rename = "serviceKey")]
    pub service_key: String,
    pub language_version: String,
    pub compiler_version: String,
    pub runtime_version: String,
    #[serde(rename = "runtimePackageVersion")]
    pub runtime_package_version: String,
    #[serde(rename = "managedStateProtocolVersion")]
    pub managed_state_protocol_version: u8,
    #[serde(rename = "managedStateLayoutVersion")]
    pub managed_state_layout_version: u8,
    #[serde(rename = "recoveryFormatVersion")]
    pub recovery_format_version: u8,
    pub abi_version: u32,
    pub target_adapter_version: String,
    pub pvm_toolchain: String,
    #[serde(rename = "rustToolchain")]
    pub rust_toolchain: String,
    #[serde(rename = "rustcVersion")]
    pub rustc_version: String,
    #[serde(rename = "polkavmLinkerVersion")]
    pub polkavm_linker_version: String,
    #[serde(rename = "polkavmTargetVariant")]
    pub polkavm_target_variant: String,
    #[serde(rename = "polkavmTargetHash")]
    pub polkavm_target_hash: String,
    #[serde(rename = "guestArchitecture")]
    pub guest_architecture: String,
    #[serde(rename = "guestAbi")]
    pub guest_abi: String,
    #[serde(rename = "cCompiler")]
    pub c_compiler: String,
    #[serde(rename = "finalElfLinker")]
    pub final_elf_linker: String,
    #[serde(rename = "finalElfLinkerOverrides")]
    pub final_elf_linker_overrides: Vec<String>,
    #[serde(rename = "targetEnvironment")]
    pub target_environment: String,
    #[serde(rename = "minimumStackBytes")]
    pub minimum_stack_bytes: u64,
    pub clang_version: String,
    pub minijam_sdk_revision: String,
    #[serde(rename = "minijamConverterRevision")]
    pub converter_revision: String,
    pub source_hash: String,
    pub abi_hash: String,
    pub code_hash: Option<String>,
    pub native_abi_version: u32,
    pub native_modules: Vec<NativeModuleMetadata>,
}

#[derive(Clone, Debug)]
pub struct NativeModule {
    pub name: String,
    pub sources: Vec<PathBuf>,
    pub include_dirs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeModuleMetadata {
    pub name: String,
    pub language: String,
    pub sources: Vec<NativeSourceMetadata>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeSourceMetadata {
    pub path: String,
    pub hash: String,
}

pub struct MiniJamTarget {
    pub sdk_root: PathBuf,
    pub converter_manifest: PathBuf,
}

impl MiniJamTarget {
    pub fn from_sdk_root(sdk_root: impl Into<PathBuf>) -> Self {
        let sdk_root = sdk_root.into();
        Self {
            converter_manifest: sdk_root
                .join("service-toolchain/compiler/polkavm-to-jam/Cargo.toml"),
            sdk_root,
        }
    }

    pub fn emit_generated_source(
        &self,
        ir: &ServiceIr,
        context: PortableServiceContext,
        output: &Path,
    ) -> Result<()> {
        fs::write(
            output,
            generate_no_std_rust_with_context(ir, context)
                .map_err(|error| anyhow::anyhow!(error))?,
        )
        .with_context(|| format!("writing {}", output.display()))?;
        Ok(())
    }

    pub fn build_probe(
        &self,
        project_root: &Path,
        ir: &ServiceIr,
        context: PortableServiceContext,
        output_dir: &Path,
        native_modules: &[NativeModule],
    ) -> Result<BuildMetadata> {
        fs::create_dir_all(output_dir)?;
        let generated = output_dir.join("generated_service.rs");
        self.emit_generated_source(ir, context, &generated)?;
        let abi = abi_for(ir);
        fs::write(
            output_dir.join("service.abi.json"),
            serde_json::to_vec_pretty(&abi)?,
        )?;
        let source_hash = hash_file(&generated)?;
        let abi_hash = hash_file(&output_dir.join("service.abi.json"))?;

        let guest_project = tempdir().context("creating Rust guest project")?;
        fs::create_dir_all(guest_project.path().join("src"))?;
        let runtime_core = workspace_crate("jamscript-runtime-core")?;
        let service_runtime_core = workspace_crate("service-runtime-core")?;
        let service_runtime_guest = workspace_crate("service-runtime-guest")?;
        let diagnostic_feature = if context.diagnostic {
            ", features = [\"diagnostic\"]"
        } else {
            ""
        };
        fs::write(guest_project.path().join("Cargo.toml"), format!(
            "[package]\nname = \"jamscript_guest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\ncrate-type = [\"cdylib\"]\n[dependencies]\njamscript-runtime-core = {{ path = \"{}\", default-features = false }}\nservice-runtime-core = {{ path = \"{}\", default-features = false }}\nservice-runtime-guest = {{ path = \"{}\", default-features = false{} }}\n[workspace]\n",
            runtime_core.display(), service_runtime_core.display(), service_runtime_guest.display(), diagnostic_feature
        ))?;
        fs::copy(&generated, guest_project.path().join("src/lib.rs"))?;
        fs::write(guest_project.path().join("build.rs"), build_script())?;

        let work = tempdir().context("creating MiniJAM native build directory")?;
        let clang = pinned_clang()?;
        let mut archives = vec![compile_sdk_archive(&self.sdk_root, &clang, work.path())?];
        for module in native_modules {
            archives.push(compile_native_archive(module, &clang, work.path())?);
        }
        let backend_output = work.path().join("polkavm");
        let artifacts = PolkaVmBuilder::new(PolkaVmBuildConfig {
            diagnostic: context.diagnostic,
            rustflags: Some(jam_rustflags()),
            ..Default::default()
        })
        .build(&PolkaVmBuildRequest {
            manifest_path: guest_project.path().join("Cargo.toml"),
            output_dir: backend_output,
            native_archives: archives,
            required_exports: vec!["minijam_refine".into(), "minijam_accumulate".into()],
            require_relocations: true,
        })?;
        fs::copy(&artifacts.elf, output_dir.join("service.elf"))?;

        let blob = output_dir.join("service.blob");
        let polkavm = output_dir.join("service.polkavm");
        let converter = self.sdk_root.join(
            "service-toolchain/compiler/polkavm-to-jam/target/release/minijam-polkavm-to-jam",
        );
        if converter.is_file() {
            let mut command = Command::new(converter);
            command.args([
                artifacts.elf.to_str().unwrap(),
                blob.to_str().unwrap(),
                polkavm.to_str().unwrap(),
            ]);
            run(
                &mut command,
                "converting the guest ELF to PolkaVM/JAM artifacts",
            )?;
        } else {
            let mut command = Command::new("cargo");
            command.args([
                "run",
                "--quiet",
                "--locked",
                "--release",
                "--manifest-path",
                self.converter_manifest.to_str().unwrap(),
                "--",
                artifacts.elf.to_str().unwrap(),
                blob.to_str().unwrap(),
                polkavm.to_str().unwrap(),
            ]);
            run(
                &mut command,
                "building and running the pinned PolkaVM converter",
            )?;
        }
        fs::copy(&polkavm, output_dir.join("service.pvm"))?;
        let clang_version = command_version(&clang)?;
        let sdk_revision = git_revision(&self.sdk_root)?;
        let native_metadata = native_metadata(project_root, native_modules)?;
        Ok(build_metadata(
            context,
            source_hash,
            abi_hash,
            hash_file(&blob)?,
            clang_version,
            sdk_revision.clone(),
            native_metadata,
            artifacts,
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn build_metadata(
    context: PortableServiceContext,
    source_hash: String,
    abi_hash: String,
    code_hash: String,
    clang_version: String,
    sdk_revision: String,
    native_modules: Vec<NativeModuleMetadata>,
    artifacts: GuestBuildArtifacts,
) -> BuildMetadata {
    let toolchain = artifacts.metadata;
    BuildMetadata {
        service_key: format!(
            "0x{}",
            context
                .service_key
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        language_version: "0.1".into(),
        compiler_version: env!("CARGO_PKG_VERSION").into(),
        runtime_version: "0.1.0".into(),
        runtime_package_version: "service-runtime-0.1.0".into(),
        managed_state_protocol_version: MANAGED_STATE_PROTOCOL_VERSION,
        managed_state_layout_version: MANAGED_STATE_LAYOUT_VERSION,
        recovery_format_version: RECOVERY_FORMAT_VERSION,
        abi_version: 1,
        target_adapter_version: "minijam-0.2".into(),
        pvm_toolchain: format!(
            "official polkavm-linker {} target + rust-lld",
            toolchain.polkavm_linker_version
        ),
        rust_toolchain: toolchain.rust_toolchain,
        rustc_version: toolchain.rustc_version,
        polkavm_linker_version: toolchain.polkavm_linker_version,
        polkavm_target_variant: toolchain.polkavm_target_variant,
        polkavm_target_hash: toolchain.polkavm_target_hash,
        guest_architecture: toolchain.guest_architecture,
        guest_abi: toolchain.guest_abi,
        c_compiler: clang_version.clone(),
        final_elf_linker: toolchain.final_elf_linker,
        final_elf_linker_overrides: vec!["-z".into(), "notext".into()],
        target_environment: toolchain.target_environment,
        minimum_stack_bytes: toolchain.minimum_stack_bytes,
        clang_version,
        minijam_sdk_revision: sdk_revision.clone(),
        converter_revision: sdk_revision,
        source_hash,
        abi_hash,
        code_hash: Some(code_hash),
        native_abi_version: NATIVE_ABI_VERSION,
        native_modules,
    }
}

fn jam_rustflags() -> String {
    jam_rustflags_from(&std::env::var("RUSTFLAGS").unwrap_or_default())
}

fn jam_rustflags_from(existing: &str) -> String {
    if existing.is_empty() {
        "-C link-arg=-z -C link-arg=notext".into()
    } else {
        format!("{existing} -C link-arg=-z -C link-arg=notext")
    }
}

#[cfg(test)]
mod linker_override_tests {
    use super::jam_rustflags_from;

    #[test]
    fn jam_metadata_relocation_override_is_explicit() {
        assert_eq!(jam_rustflags_from(""), "-C link-arg=-z -C link-arg=notext");
        assert_eq!(
            jam_rustflags_from("-C opt-level=2"),
            "-C opt-level=2 -C link-arg=-z -C link-arg=notext"
        );
    }
}

fn workspace_crate(name: &str) -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name)
        .canonicalize()
        .with_context(|| format!("locating workspace crate {name}"))
}

fn build_script() -> &'static str {
    r#"fn main() {
    let value = std::env::var("SERVICE_BUILD_POLKAVM_NATIVE_ARCHIVES").unwrap_or_default();
    for entry in value.lines() {
        let Some((name, path)) = entry.split_once('=') else { continue };
        let path = std::path::Path::new(path);
        if let Some(parent) = path.parent() { println!("cargo:rustc-link-search=native={}", parent.display()); }
        println!("cargo:rustc-link-lib=static={name}");
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
"#
}

fn pinned_clang() -> Result<PathBuf> {
    let clang = std::env::var_os("JAMSCRIPT_CLANG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/clang"));
    if !clang.is_file() {
        bail!(
            "pinned Clang 20 was not found at {}; set JAMSCRIPT_CLANG",
            clang.display()
        );
    }
    Ok(clang)
}

fn compile_sdk_archive(sdk_root: &Path, clang: &Path, work: &Path) -> Result<NativeArchive> {
    let include = sdk_root.join("service-toolchain/sdk/include");
    let source_root = sdk_root.join("service-toolchain/sdk/src");
    let sources = ["host", "minijam", "crypto"].map(|unit| source_root.join(format!("{unit}.c")));
    compile_archive("minijam_guest", &sources, &[include], clang, work)
}

fn compile_native_archive(
    module: &NativeModule,
    clang: &Path,
    work: &Path,
) -> Result<NativeArchive> {
    compile_archive(
        &format!("native_{}", module.name),
        &module.sources,
        &module.include_dirs,
        clang,
        work,
    )
}

fn compile_archive(
    name: &str,
    sources: &[PathBuf],
    include_dirs: &[PathBuf],
    clang: &Path,
    work: &Path,
) -> Result<NativeArchive> {
    let common = [
        "--target=riscv64-unknown-elf",
        "-march=rv64emac",
        "-mabi=lp64e",
        "-ffreestanding",
        "-fno-builtin",
        "-fPIC",
        "-fdata-sections",
        "-ffunction-sections",
        "-Os",
        "-Wall",
        "-Wextra",
        "-Werror",
    ];
    let mut objects = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let object = work.join(format!("{name}_{index}.o"));
        let mut command = Command::new(clang);
        command.args(common).arg("-std=c11");
        for include in include_dirs {
            command.arg("-I").arg(include);
        }
        command.args([
            "-c",
            source.to_str().unwrap(),
            "-o",
            object.to_str().unwrap(),
        ]);
        run(&mut command, &format!("compiling {}", source.display()))?;
        objects.push(object);
    }
    let ar = std::env::var_os("JAMSCRIPT_LLVM_AR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/llvm-ar"));
    let ar = if ar.is_file() {
        ar
    } else {
        PathBuf::from("ar")
    };
    let archive = work.join(format!("lib{name}.a"));
    let mut command = Command::new(ar);
    command.arg("crs").arg(&archive).args(&objects);
    run(&mut command, &format!("archiving {name}"))?;
    Ok(NativeArchive {
        name: name.into(),
        path: archive,
    })
}

fn native_metadata(
    project_root: &Path,
    modules: &[NativeModule],
) -> Result<Vec<NativeModuleMetadata>> {
    modules
        .iter()
        .map(|module| {
            Ok(NativeModuleMetadata {
                name: module.name.clone(),
                language: "c".into(),
                sources: module
                    .sources
                    .iter()
                    .map(|source| {
                        Ok(NativeSourceMetadata {
                            path: source
                                .strip_prefix(project_root)
                                .unwrap_or(source)
                                .to_string_lossy()
                                .replace('\\', "/"),
                            hash: hash_file(source)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            })
        })
        .collect()
}

fn command_version(command: &Path) -> Result<String> {
    let output = Command::new(command).arg("--version").output()?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string())
}

fn run(command: &mut Command, description: &str) -> Result<()> {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("{description}: start command"))?;
    if !output.status.success() {
        bail!(
            "{description} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(format!(
        "0x{}",
        blake2b_simd::Params::new()
            .hash_length(32)
            .hash(&bytes)
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

fn git_revision(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", path.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .with_context(|| format!("reading git revision for {}", path.display()))?;
    if !output.status.success() {
        bail!("{} is not a git checkout", path.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn target_does_not_contain_a_clang_final_link() {
        let source = include_str!("lib.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        assert!(!implementation.contains("Wl,--gc-sections"));
        assert!(source.contains("crate-type = [\\\"cdylib\\\"]"));
        assert!(source.contains("final_elf_linker"));
    }
}
