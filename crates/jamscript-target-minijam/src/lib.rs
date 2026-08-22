use anyhow::{bail, Context, Result};
use jamscript_codegen_rust::{generate_no_std_rust_with_context, MiniJamContext};
use jamscript_ir::{abi_for, ServiceIr, NATIVE_ABI_VERSION};
use serde::Serialize;
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
    pub rust_toolchain: String,
    pub clang_version: String,
    pub minijam_sdk_revision: String,
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
    pub rust_target: PathBuf,
}

impl MiniJamTarget {
    pub fn from_sdk_root(sdk_root: impl Into<PathBuf>) -> Self {
        let sdk_root = sdk_root.into();
        Self {
            converter_manifest: sdk_root
                .join("service-toolchain/compiler/polkavm-to-jam/Cargo.toml"),
            sdk_root,
            rust_target: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../toolchains/riscv64emac-unknown-none.json"),
        }
    }

    pub fn emit_generated_source(
        &self,
        ir: &ServiceIr,
        context: MiniJamContext,
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
        context: MiniJamContext,
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
        let object = output_dir.join("generated_service.o");
        let guest_project = tempdir().context("creating Rust guest project")?;
        fs::create_dir_all(guest_project.path().join("src"))?;
        let runtime_core = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../jamscript-runtime-core")
            .canonicalize()?;
        let service_runtime_core = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../service-runtime-core")
            .canonicalize()?;
        let service_runtime_guest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../service-runtime-guest")
            .canonicalize()?;
        fs::write(
            guest_project.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"jamscript_guest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\ncrate-type = [\"staticlib\"]\n[dependencies]\njamscript-runtime-core = {{ path = \"{}\", default-features = false }}\nservice-runtime-core = {{ path = \"{}\", default-features = false }}\nservice-runtime-guest = {{ path = \"{}\", default-features = false }}\n",
                runtime_core.display(),
                service_runtime_core.display(),
                service_runtime_guest.display()
            ),
        )?;
        fs::copy(&generated, guest_project.path().join("src/lib.rs"))?;
        let mut rust_build = Command::new("cargo");
        rust_build.args([
            "+nightly-2026-05-02",
            "-Z",
            "build-std=core,alloc",
            "-Z",
            "json-target-spec",
            "build",
            "--release",
            "--target",
            self.rust_target.to_str().unwrap(),
            "--manifest-path",
            guest_project.path().join("Cargo.toml").to_str().unwrap(),
            "--offline",
            "-p",
            "jamscript_guest",
        ]);
        let status = rust_build
            .status()
            .context("starting cargo for the Rust guest")?;
        if !status.success() {
            bail!(
                "Rust PVM backend unavailable: rustc could not compile target {}",
                self.rust_target.display()
            );
        }
        let guest_library = guest_project
            .path()
            .join("target")
            .join("riscv64emac-unknown-none")
            .join("release")
            .join("libjamscript_guest.a");
        if !guest_library.is_file() {
            bail!("Rust guest build did not emit {}", guest_library.display());
        }
        fs::copy(guest_library, &object)?;
        let clang = std::env::var_os("JAMSCRIPT_CLANG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/clang"));
        if !clang.is_file() {
            bail!(
                "MiniJAM PVM backend unavailable: pinned Clang 20 was not found at {}; set JAMSCRIPT_CLANG to a compatible Clang 20 binary",
                clang.display()
            );
        }
        let work = tempdir().context("creating MiniJAM guest build directory")?;
        let include = self.sdk_root.join("service-toolchain/sdk/include");
        let sdk_src = self.sdk_root.join("service-toolchain/sdk/src");
        let common = [
            "--target=riscv64-unknown-elf",
            "-march=rv64emac",
            "-mabi=lp64e",
            "-ffreestanding",
            "-fno-builtin",
            "-fdata-sections",
            "-ffunction-sections",
            "-Os",
            "-Wall",
            "-Wextra",
            "-Werror",
        ];
        for unit in ["host", "minijam", "crypto"] {
            let object = work.path().join(format!("{unit}.o"));
            let source = sdk_src.join(format!("{unit}.c"));
            let mut command = Command::new(&clang);
            command
                .args(common)
                .arg("-I")
                .arg(&include)
                .arg("-c")
                .arg(&source)
                .arg("-o")
                .arg(&object);
            run(&mut command, &format!("compiling MiniJAM SDK {unit}.c"))?;
        }
        let mut native_objects = Vec::new();
        for module in native_modules {
            for (index, source) in module.sources.iter().enumerate() {
                let object = work
                    .path()
                    .join(format!("native_{}_{}.o", module.name, index));
                let mut command = Command::new(&clang);
                command.args(common).arg("-std=c11");
                for include_dir in &module.include_dirs {
                    command.arg("-I").arg(include_dir);
                }
                command.arg("-c").arg(source).arg("-o").arg(&object);
                run(
                    &mut command,
                    &format!(
                        "compiling native module {} source {}",
                        module.name,
                        source.display()
                    ),
                )?;
                native_objects.push(object);
            }
        }
        let elf = work.path().join("service.elf");
        let mut link = Command::new(&clang);
        link.args([
            "--target=riscv64-unknown-elf",
            "-march=rv64emac",
            "-mabi=lp64e",
            "-nostdlib",
            "-Wl,--gc-sections",
            "-Wl,--emit-relocs",
            "-Wl,-e,minijam_refine",
            "-Wl,-u,minijam_accumulate",
        ]);
        for unit in ["host", "minijam", "crypto"] {
            link.arg(work.path().join(format!("{unit}.o")));
        }
        for object in &native_objects {
            link.arg(object);
        }
        link.arg(&object).arg("-o").arg(&elf);
        run(&mut link, "linking Rust guest with the MiniJAM SDK")?;

        let blob = output_dir.join("service.blob");
        let polkavm = output_dir.join("service.polkavm");
        let converter = self.sdk_root.join(
            "service-toolchain/compiler/polkavm-to-jam/target/release/minijam-polkavm-to-jam",
        );
        if converter.is_file() {
            let mut command = Command::new(converter);
            command.args([
                elf.to_str().unwrap(),
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
                elf.to_str().unwrap(),
                blob.to_str().unwrap(),
                polkavm.to_str().unwrap(),
            ]);
            run(
                &mut command,
                "building and running the pinned PolkaVM converter",
            )?;
        }
        fs::copy(&polkavm, output_dir.join("service.pvm"))?;
        let clang_version = Command::new(&clang)
            .arg("--version")
            .output()
            .context("reading Clang version")?;
        let clang_version = String::from_utf8_lossy(&clang_version.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let sdk_revision = git_revision(&self.sdk_root)?;
        let native_modules = native_modules
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
            .collect::<Result<Vec<_>>>()?;
        Ok(BuildMetadata {
            language_version: "0.1".into(),
            compiler_version: env!("CARGO_PKG_VERSION").into(),
            runtime_version: "0.1.0".into(),
            runtime_package_version: "service-runtime-0.1.0".into(),
            managed_state_protocol_version: MANAGED_STATE_PROTOCOL_VERSION,
            managed_state_layout_version: MANAGED_STATE_LAYOUT_VERSION,
            recovery_format_version: RECOVERY_FORMAT_VERSION,
            abi_version: 1,
            target_adapter_version: "minijam-0.1".into(),
            pvm_toolchain:
                "custom riscv64emac/lp64e Rust target + clang 20 / polkavm-linker-0.30.0".into(),
            rust_toolchain: "nightly-2026-05-02".into(),
            clang_version,
            minijam_sdk_revision: sdk_revision.clone(),
            converter_revision: sdk_revision,
            source_hash,
            abi_hash,
            code_hash: Some(hash_file(&blob)?),
            native_abi_version: NATIVE_ABI_VERSION,
            native_modules,
        })
    }
}

fn run(command: &mut Command, description: &str) -> Result<()> {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("{description}: start command"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{description} failed: {}", stderr.trim());
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
