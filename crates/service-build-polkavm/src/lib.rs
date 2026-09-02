use anyhow::{bail, Context, Result};
use polkavm_linker::{target_json_path, TargetJsonArgs};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug)]
pub struct PolkaVmBuildConfig {
    pub rust_toolchain: String,
    pub is_64_bit: bool,
    pub diagnostic: bool,
    pub lock_path: PathBuf,
    pub rustflags: Option<String>,
    /// Managed distributions pass these explicitly. `None` is retained only
    /// for contributor-facing callers that use rustup.
    pub cargo_path: Option<PathBuf>,
    pub rustc_path: Option<PathBuf>,
    pub cargo_home: Option<PathBuf>,
}

impl Default for PolkaVmBuildConfig {
    fn default() -> Self {
        Self {
            rust_toolchain: default_toolchain(),
            is_64_bit: true,
            diagnostic: false,
            lock_path: repo_root().join("toolchains/polkavm.lock"),
            rustflags: None,
            cargo_path: None,
            rustc_path: None,
            cargo_home: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeArchive {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct PolkaVmBuildRequest {
    pub manifest_path: PathBuf,
    pub output_dir: PathBuf,
    pub native_archives: Vec<NativeArchive>,
    pub required_exports: Vec<String>,
    pub require_relocations: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GuestToolchainMetadata {
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
    pub c_compiler: Option<String>,
    #[serde(rename = "finalElfLinker")]
    pub final_elf_linker: String,
    #[serde(rename = "targetEnvironment")]
    pub target_environment: String,
    #[serde(rename = "minimumStackBytes")]
    pub minimum_stack_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct GuestBuildArtifacts {
    pub elf: PathBuf,
    pub target_json: PathBuf,
    pub metadata: GuestToolchainMetadata,
}

pub struct PolkaVmBuilder {
    pub config: PolkaVmBuildConfig,
}

impl PolkaVmBuilder {
    pub fn new(config: PolkaVmBuildConfig) -> Self {
        Self { config }
    }

    pub fn build(&self, request: &PolkaVmBuildRequest) -> Result<GuestBuildArtifacts> {
        let lock = ToolchainLock::load(&self.config.lock_path)?;
        if self.config.rust_toolchain != lock.rust {
            bail!(
                "configured Rust toolchain {} disagrees with {}: {}",
                self.config.rust_toolchain,
                self.config.lock_path.display(),
                lock.rust
            );
        }
        if !request.manifest_path.is_file() {
            bail!(
                "guest manifest does not exist: {}",
                request.manifest_path.display()
            );
        }
        if !self.config.is_64_bit {
            bail!(
                "PolkaVM service backend currently supports only the pinned riscv64/lp64e domain"
            );
        }
        fs::create_dir_all(&request.output_dir)?;

        // 0.30's non-exhaustive default is deliberately the public API for
        // `is_64_bit = true` plus RustcVersion::Autodetect. Do not infer a
        // target variant from the installed compiler here.
        let target_json = target_json_path(TargetJsonArgs::default())
            .map_err(|error| anyhow::anyhow!("selecting official PolkaVM target: {error}"))?
            .canonicalize()
            .context("canonicalizing official PolkaVM target")?;
        let target_variant = target_json
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
            .to_string();
        let target_name = target_json
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow::anyhow!("official PolkaVM target has no valid file name"))?
            .trim_end_matches(".json")
            .to_string();
        let target_hash = hash_file(&target_json)?;

        let target_dir = request
            .manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("target-polkavm");
        let mut cargo = Command::new(
            self.config
                .cargo_path
                .as_deref()
                .unwrap_or_else(|| Path::new("cargo")),
        );
        if self.config.cargo_path.is_none() {
            cargo.arg(format!("+{}", self.config.rust_toolchain));
        }
        cargo
            .args([
                "-Z",
                "build-std=core,alloc",
                "-Z",
                "json-target-spec",
                "build",
                "--release",
                "--target",
                target_json.to_str().unwrap(),
                "--target-dir",
                target_dir.to_str().unwrap(),
                "--manifest-path",
                request.manifest_path.to_str().unwrap(),
                "--offline",
            ])
            .env(
                "SERVICE_BUILD_POLKAVM_NATIVE_ARCHIVES",
                encode_archives(&request.native_archives),
            );
        if let Some(rustc) = &self.config.rustc_path {
            cargo.env("RUSTC", rustc);
        }
        if let Some(cargo_home) = &self.config.cargo_home {
            cargo.env("CARGO_HOME", cargo_home);
        }
        if let Some(rustflags) = &self.config.rustflags {
            cargo.env("RUSTFLAGS", rustflags);
        }
        if self.config.diagnostic {
            cargo.env("JAMSCRIPT_DIAGNOSTIC_GUEST", "1");
        }
        run(&mut cargo, "building the PolkaVM cdylib guest")?;

        let release_dir = target_dir.join(&target_name).join("release");
        let elf = find_single_elf(&release_dir)?;
        let output_elf = request.output_dir.join("service.elf");
        fs::copy(&elf, &output_elf)
            .with_context(|| format!("copying canonical guest ELF from {}", elf.display()))?;
        let diagnostics = validate_elf(
            &output_elf,
            &request.required_exports,
            request.require_relocations,
        )?;
        if self.config.diagnostic {
            fs::write(request.output_dir.join("readelf.txt"), &diagnostics.header)?;
            fs::write(
                request.output_dir.join("relocations.txt"),
                &diagnostics.relocations,
            )?;
            fs::write(request.output_dir.join("symbols.txt"), &diagnostics.symbols)?;
        }

        let rustc_version = match &self.config.rustc_path {
            Some(rustc) => command_rustc_version(rustc)?,
            None => rustc_version(&self.config.rust_toolchain)?,
        };
        validate_resolved_guest_versions(&request.manifest_path, &lock)?;
        Ok(GuestBuildArtifacts {
            elf: output_elf,
            target_json,
            metadata: GuestToolchainMetadata {
                rust_toolchain: self.config.rust_toolchain.clone(),
                rustc_version,
                polkavm_linker_version: lock.polkavm_linker,
                polkavm_target_variant: target_variant,
                polkavm_target_hash: target_hash,
                guest_architecture: if self.config.is_64_bit {
                    "riscv64".into()
                } else {
                    "riscv32".into()
                },
                guest_abi: "lp64e".into(),
                c_compiler: None,
                final_elf_linker: "rust-lld".into(),
                target_environment: "polkavm".into(),
                minimum_stack_bytes: 2 * 1024 * 1024,
            },
        })
    }
}

struct ElfDiagnostics {
    header: String,
    relocations: String,
    symbols: String,
}

fn validate_elf(
    path: &Path,
    required_exports: &[String],
    require_relocations: bool,
) -> Result<ElfDiagnostics> {
    let readelf = env::var_os("JAMSCRIPT_READELF")
        .map(PathBuf::from)
        .or_else(|| find_on_path("readelf"))
        .or_else(|| find_on_path("llvm-readelf"))
        .ok_or_else(|| anyhow::anyhow!("ELF validation requires readelf or llvm-readelf"))?;
    let header = readelf_output(&readelf, path, &["-hW"])?;
    let relocations = readelf_output(&readelf, path, &["-rW"])?;
    let symbols = readelf_output(&readelf, path, &["-sW"])?;
    if !header.contains("RISC-V") {
        bail!("canonical guest ELF is not RISC-V: {}", path.display());
    }
    if !header.contains("RVE") {
        bail!(
            "canonical guest ELF is not lp64e/RVE compatible: {}",
            path.display()
        );
    }
    if !header.contains("DYN (Shared object file)") {
        bail!(
            "canonical guest ELF is not a PIE/shared object: {}",
            path.display()
        );
    }
    for export in required_exports {
        if !symbols.contains(export) {
            bail!("canonical guest ELF is missing required export {export}");
        }
    }
    let forbidden_undefined = ["libc", "malloc", "calloc", "realloc", "free", "pthread"];
    let forbidden_symbols = symbols
        .lines()
        .filter(|line| {
            line.contains(" UND ") && forbidden_undefined.iter().any(|name| line.contains(name))
        })
        .collect::<Vec<_>>();
    if !forbidden_symbols.is_empty() {
        bail!(
            "canonical guest ELF contains undefined system/libc symbols: {}",
            forbidden_symbols.join(" | ")
        );
    }
    if require_relocations && relocations.contains("There are no relocations") {
        bail!("canonical guest ELF has no PolkaVM relocations");
    }
    Ok(ElfDiagnostics {
        header,
        relocations,
        symbols,
    })
}

fn readelf_output(readelf: &Path, path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(readelf)
        .args(args)
        .arg(path)
        .output()
        .with_context(|| format!("running {}", readelf.display()))?;
    if !output.status.success() {
        bail!(
            "{} failed for {}: {}",
            readelf.display(),
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn find_single_elf(release_dir: &Path) -> Result<PathBuf> {
    let mut candidates = fs::read_dir(release_dir)
        .with_context(|| format!("reading {}", release_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("elf"))
        .collect::<Vec<_>>();
    candidates.sort();
    match candidates.as_slice() {
        [elf] => Ok(elf.clone()),
        [] => bail!(
            "PolkaVM cdylib build emitted no .elf in {}",
            release_dir.display()
        ),
        _ => bail!(
            "PolkaVM cdylib build emitted multiple .elf files in {}",
            release_dir.display()
        ),
    }
}

fn encode_archives(archives: &[NativeArchive]) -> String {
    archives
        .iter()
        .map(|archive| format!("{}={}", archive.name, archive.path.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rustc_version(toolchain: &str) -> Result<String> {
    let output = Command::new("rustup")
        .args(["run", toolchain, "rustc", "--version"])
        .output()
        .with_context(|| format!("reading rustc version for {toolchain}"))?;
    if !output.status.success() {
        bail!(
            "rustc version query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn command_rustc_version(rustc: &Path) -> Result<String> {
    let output = Command::new(rustc)
        .arg("--version")
        .output()
        .with_context(|| format!("reading rustc version from {}", rustc.display()))?;
    if !output.status.success() {
        bail!(
            "rustc version query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

fn find_on_path(program: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(program))
            .find(|candidate| candidate.is_file())
    })
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
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[derive(Clone, Debug, Deserialize)]
struct ToolchainLock {
    version: u32,
    rust: String,
    polkavm_linker: String,
    polkavm_derive: String,
    architecture: String,
    abi: String,
    target_source: String,
    target_selection: String,
    clang_major: u32,
    #[serde(default)]
    clang_version: Option<String>,
}

impl ToolchainLock {
    fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("reading PolkaVM toolchain lock {}", path.display()))?;
        let lock: Self = toml::from_str(&contents)
            .with_context(|| format!("parsing PolkaVM toolchain lock {}", path.display()))?;
        if lock.version != 1
            || lock.architecture != "riscv64"
            || lock.abi != "lp64e"
            || lock.target_source != "polkavm-linker"
            || lock.target_selection != "autodetect"
            || lock.clang_major != 20
            || lock.clang_version.as_deref() != Some("20.1.8")
        {
            bail!("unsupported PolkaVM toolchain lock {}", path.display());
        }
        let cargo_lock = path
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("Cargo.lock"))
            .ok_or_else(|| anyhow::anyhow!("cannot locate Cargo.lock beside {}", path.display()))?;
        validate_lock_package_versions(
            &cargo_lock,
            &[
                ("polkavm-linker", &lock.polkavm_linker),
                ("polkavm-derive", &lock.polkavm_derive),
            ],
        )?;
        Ok(lock)
    }
}

fn validate_resolved_guest_versions(manifest: &Path, lock: &ToolchainLock) -> Result<()> {
    let cargo_lock = manifest
        .parent()
        .map(|dir| dir.join("Cargo.lock"))
        .ok_or_else(|| anyhow::anyhow!("guest manifest has no parent directory"))?;
    if cargo_lock.is_file() {
        validate_lock_package_versions(&cargo_lock, &[("polkavm-derive", &lock.polkavm_derive)])?;
    }
    Ok(())
}

fn validate_lock_package_versions(path: &Path, expected: &[(&str, &str)]) -> Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("reading Cargo lock {}", path.display()))?;
    let value: toml::Value = toml::from_str(&contents)
        .with_context(|| format!("parsing Cargo lock {}", path.display()))?;
    let packages = value
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Cargo lock {} has no package table", path.display()))?;
    for (name, expected_version) in expected {
        let versions = packages
            .iter()
            .filter(|package| package.get("name").and_then(toml::Value::as_str) == Some(*name))
            .filter_map(|package| package.get("version").and_then(toml::Value::as_str))
            .collect::<Vec<_>>();
        if !versions.contains(expected_version) {
            bail!(
                "Cargo lock {} resolves {} as {:?}, expected {}",
                path.display(),
                name,
                versions,
                expected_version
            );
        }
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn default_toolchain() -> String {
    let path = repo_root().join("toolchains/polkavm.lock");
    fs::read_to_string(&path)
        .ok()
        .and_then(|contents| toml::from_str::<ToolchainLock>(&contents).ok())
        .map(|lock| lock.rust)
        .unwrap_or_else(|| "nightly-2026-05-02".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_environment_is_deterministic() {
        assert_eq!(
            encode_archives(&[
                NativeArchive {
                    name: "minijam_guest".into(),
                    path: PathBuf::from("/tmp/libminijam_guest.a"),
                },
                NativeArchive {
                    name: "native_game".into(),
                    path: PathBuf::from("/tmp/libnative_game.a"),
                },
            ]),
            "minijam_guest=/tmp/libminijam_guest.a\nnative_game=/tmp/libnative_game.a"
        );
    }

    #[test]
    fn toolchain_lock_is_the_build_source_of_truth() {
        let lock = ToolchainLock::load(&repo_root().join("toolchains/polkavm.lock")).unwrap();
        assert_eq!(lock.rust, PolkaVmBuildConfig::default().rust_toolchain);
        assert_eq!(lock.polkavm_linker, "0.30.0");
        assert_eq!(lock.polkavm_derive, "0.30.0");
    }
}
