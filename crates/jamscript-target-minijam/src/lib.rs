use anyhow::{bail, Context, Result};
use jamscript_codegen_rust::generate_no_std_rust;
use jamscript_ir::{abi_for, ServiceIr};
use serde::Serialize;
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
    pub abi_version: u32,
    pub target_adapter_version: String,
    pub pvm_toolchain: String,
    pub source_hash: String,
    pub abi_hash: String,
    pub code_hash: Option<String>,
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

    pub fn emit_generated_source(&self, ir: &ServiceIr, output: &Path) -> Result<()> {
        fs::write(
            output,
            generate_no_std_rust(ir).map_err(|error| anyhow::anyhow!(error))?,
        )
        .with_context(|| format!("writing {}", output.display()))?;
        Ok(())
    }

    pub fn build_probe(&self, ir: &ServiceIr, output_dir: &Path) -> Result<BuildMetadata> {
        fs::create_dir_all(output_dir)?;
        let generated = output_dir.join("generated_service.rs");
        self.emit_generated_source(ir, &generated)?;
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
        fs::write(
            guest_project.path().join("Cargo.toml"),
            "[package]\nname = \"jamscript_guest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\ncrate-type = [\"lib\"]\n",
        )?;
        fs::copy(&generated, guest_project.path().join("src/lib.rs"))?;
        let mut rust_build = Command::new("cargo");
        rust_build.args([
            "+nightly",
            "-Z",
            "build-std=core",
            "-Z",
            "json-target-spec",
            "rustc",
            "--release",
            "--target",
            self.rust_target.to_str().unwrap(),
            "--manifest-path",
            guest_project.path().join("Cargo.toml").to_str().unwrap(),
            "--",
            "-C",
            "opt-level=z",
            "-C",
            "panic=abort",
            "--emit=obj",
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
        let guest_target = guest_project
            .path()
            .join("target")
            .join("riscv64emac-unknown-none")
            .join("release")
            .join("deps");
        let guest_object = fs::read_dir(&guest_target)
            .with_context(|| format!("reading Rust guest output {}", guest_target.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| path.extension().is_some_and(|extension| extension == "o"))
            .ok_or_else(|| anyhow::anyhow!("Rust guest build did not emit an object file"))?;
        fs::copy(guest_object, &object)?;
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
        Ok(BuildMetadata {
            language_version: "0.1".into(),
            compiler_version: env!("CARGO_PKG_VERSION").into(),
            runtime_version: "0.1.0".into(),
            abi_version: 1,
            target_adapter_version: "minijam-0.1".into(),
            pvm_toolchain:
                "custom riscv64emac/lp64e Rust target + clang 20 / polkavm-linker-0.30.0".into(),
            source_hash,
            abi_hash,
            code_hash: Some(hash_file(&blob)?),
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
