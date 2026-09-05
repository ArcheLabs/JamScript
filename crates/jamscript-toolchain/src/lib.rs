//! Managed JamScript compiler distributions.
//!
//! The manager deliberately owns every executable used by a canonical build.
//! It never searches PATH for a compiler and it never falls back to a system
//! installation after a managed installation has been selected.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const DISTRIBUTION_MANIFEST: &str = include_str!("../../../toolchains/distribution-v1.toml");

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DistributionManifest {
    pub format: u32,
    pub toolchain_id: String,
    pub language: String,
    pub backend: String,
    pub node_version: String,
    pub rust_toolchain: String,
    pub clang_version: String,
    pub polkavm_linker: String,
    pub jam_target_version: String,
    pub jam_blob_encoder_version: String,
    #[serde(default)]
    pub scriptc_revision: String,
    pub platforms: BTreeMap<String, PlatformBundle>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlatformBundle {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub archive: String,
    #[serde(default = "published_by_default")]
    pub published: bool,
}

fn published_by_default() -> bool {
    true
}

#[derive(Clone, Debug, Serialize)]
pub struct InstalledToolchain {
    pub root: PathBuf,
    pub node: PathBuf,
    pub clang: PathBuf,
    pub llvm_ar: PathBuf,
    pub lld: PathBuf,
    pub rustc: PathBuf,
    pub cargo: PathBuf,
    pub scriptc: PathBuf,
    pub runtime: PathBuf,
    pub runtime_scriptc: PathBuf,
    pub cargo_home: PathBuf,
    pub polkavm_lock: PathBuf,
    pub jam_target: PathBuf,
    pub toolchain_id: String,
    pub platform: String,
    pub bundle_sha256: String,
    pub rust_toolchain: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolchainStatus {
    pub installed: bool,
    pub verified: bool,
    #[serde(rename = "toolchainId")]
    pub toolchain_id: String,
    pub platform: String,
    pub sha256: String,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InternalManifest {
    format: u32,
    toolchain_id: String,
    platform: String,
    node_version: String,
    clang_version: String,
    rust_toolchain: String,
    jam_target_version: String,
    jam_blob_encoder_version: String,
    #[serde(default)]
    scriptc_revision: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

pub struct ToolchainManager {
    manifest: DistributionManifest,
    platform: String,
    cache_home: PathBuf,
}

impl ToolchainManager {
    /// Load the distribution manifest embedded in the JamScript CLI.
    pub fn new() -> Result<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self::from_manifest_str(DISTRIBUTION_MANIFEST, &root)
    }

    pub fn from_manifest_path(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading toolchain manifest {}", path.display()))?;
        let root = path
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."));
        Self::from_manifest_str(&text, root)
    }

    pub fn from_manifest_str(text: &str, source_root: &Path) -> Result<Self> {
        let manifest: DistributionManifest =
            toml::from_str(text).context("parsing JamScript toolchain distribution manifest")?;
        validate_manifest(&manifest)?;
        validate_source_locks(&manifest, source_root)?;
        let platform = current_platform()?;
        let cache_home = cache_home()?;
        Ok(Self {
            manifest,
            platform,
            cache_home,
        })
    }

    pub fn manifest(&self) -> &DistributionManifest {
        &self.manifest
    }
    pub fn platform(&self) -> &str {
        &self.platform
    }
    pub fn with_cache_home(mut self, cache_home: impl Into<PathBuf>) -> Self {
        self.cache_home = cache_home.into();
        self
    }

    pub fn install(&self) -> Result<InstalledToolchain> {
        let bundle = self.bundle()?;
        let destination = self.install_root(bundle);
        if destination.join("manifest.json").is_file() {
            return self
                .verify_at(&destination, bundle)
                .map(|_| self.installed(&destination, bundle));
        }
        fs::create_dir_all(&self.cache_home)
            .with_context(|| format!("creating toolchain cache {}", self.cache_home.display()))?;
        let _lock = InstallLock::acquire(&self.cache_home)?;
        if destination.join("manifest.json").is_file() {
            return self
                .verify_at(&destination, bundle)
                .map(|_| self.installed(&destination, bundle));
        }
        let token = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let archive_path = self.cache_home.join(format!(
            ".download-{}-{token}.{}",
            std::process::id(),
            bundle.archive
        ));
        let staging = self
            .cache_home
            .join(format!(".tmp-{}-{token}", std::process::id()));
        let result = (|| {
            download(&bundle.url, &archive_path)?;
            verify_archive(&archive_path, bundle)?;
            fs::create_dir_all(&staging)?;
            extract_archive(&archive_path, &staging, &bundle.archive)?;
            let root = normalize_archive_root(&staging)?;
            self.verify_at(&root, bundle)?;
            if destination.exists() {
                bail!(
                    "toolchain cache destination appeared during install: {}",
                    destination.display()
                );
            }
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::rename(&root, &destination).with_context(|| {
                format!(
                    "atomically installing toolchain into {}",
                    destination.display()
                )
            })?;
            Ok(self.installed(&destination, bundle))
        })();
        let _ = fs::remove_file(&archive_path);
        let _ = fs::remove_dir_all(&staging);
        result
    }

    pub fn resolve(&self) -> Result<InstalledToolchain> {
        let bundle = self.bundle()?;
        let destination = self.install_root(bundle);
        if destination.join("manifest.json").is_file() {
            return self
                .verify_at(&destination, bundle)
                .map(|_| self.installed(&destination, bundle));
        }
        if offline() {
            bail!(
                "JamScript toolchain is not installed for {}; offline mode forbids downloading it",
                self.platform
            );
        }
        self.install()
    }

    pub fn verify(&self) -> Result<InstalledToolchain> {
        let bundle = self.bundle()?;
        let destination = self.install_root(bundle);
        self.verify_at(&destination, bundle)
            .map(|_| self.installed(&destination, bundle))
    }

    pub fn path(&self) -> Result<PathBuf> {
        let bundle = self.bundle()?;
        Ok(self.install_root(bundle))
    }

    pub fn status(&self) -> ToolchainStatus {
        let bundle = match self.manifest.platforms.get(&self.platform) {
            Some(bundle) => bundle,
            None => {
                return ToolchainStatus {
                    installed: false,
                    verified: false,
                    toolchain_id: self.manifest.toolchain_id.clone(),
                    platform: self.platform.clone(),
                    sha256: String::new(),
                    path: None,
                    error: Some(format!("unsupported host platform {}", self.platform)),
                }
            }
        };
        if !bundle.published {
            return ToolchainStatus {
                installed: false,
                verified: false,
                toolchain_id: self.manifest.toolchain_id.clone(),
                platform: self.platform.clone(),
                sha256: bundle.sha256.clone(),
                path: None,
                error: Some(format!(
                    "JamScript toolchain for {} is not yet published",
                    self.platform
                )),
            };
        }
        let root = self.install_root(bundle);
        let installed = root.join("manifest.json").is_file();
        let verification = if installed {
            self.verify_at(&root, bundle).map(|_| ())
        } else {
            Err(anyhow::anyhow!("not installed"))
        };
        ToolchainStatus {
            installed,
            verified: verification.is_ok(),
            toolchain_id: self.manifest.toolchain_id.clone(),
            platform: self.platform.clone(),
            sha256: bundle.sha256.clone(),
            path: installed.then_some(root),
            error: verification.err().map(|error| error.to_string()),
        }
    }

    fn bundle(&self) -> Result<&PlatformBundle> {
        let bundle = self.manifest.platforms.get(&self.platform).ok_or_else(|| {
            anyhow::anyhow!(
                "JamScript toolchain for {} is not yet published",
                self.platform
            )
        })?;
        if !bundle.published {
            bail!(
                "JamScript toolchain for {} is not yet published",
                self.platform
            );
        }
        Ok(bundle)
    }

    fn install_root(&self, bundle: &PlatformBundle) -> PathBuf {
        self.cache_home
            .join(&self.manifest.toolchain_id)
            .join(&self.platform)
            .join(normalize_hash(&bundle.sha256))
    }

    fn installed(&self, root: &Path, bundle: &PlatformBundle) -> InstalledToolchain {
        let executable = |name: &str| {
            if self.platform.starts_with("windows-") {
                root.join("bin").join(format!("{name}.exe"))
            } else {
                root.join("bin").join(name)
            }
        };
        InstalledToolchain {
            root: root.to_path_buf(),
            node: executable("node"),
            clang: executable("clang"),
            llvm_ar: executable("llvm-ar"),
            lld: executable("ld.lld"),
            rustc: executable("rustc"),
            cargo: executable("cargo"),
            scriptc: root.join("scriptc"),
            runtime: root.join("runtime"),
            runtime_scriptc: root.join("runtime-scriptc"),
            cargo_home: root.join("cargo"),
            polkavm_lock: root.join("toolchains/polkavm.lock"),
            jam_target: root.join("targets/jam/sdk"),
            toolchain_id: self.manifest.toolchain_id.clone(),
            platform: self.platform.clone(),
            bundle_sha256: bundle.sha256.clone(),
            rust_toolchain: self.manifest.rust_toolchain.clone(),
        }
    }

    fn verify_at(&self, root: &Path, bundle: &PlatformBundle) -> Result<()> {
        let path = root.join("manifest.json");
        let internal: InternalManifest = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("decoding {}", path.display()))?;
        if internal.format != self.manifest.format
            || internal.toolchain_id != self.manifest.toolchain_id
            || internal.platform != self.platform
            || internal.node_version != self.manifest.node_version
            || internal.clang_version != self.manifest.clang_version
            || internal.rust_toolchain != self.manifest.rust_toolchain
            || internal.jam_target_version != self.manifest.jam_target_version
            || internal.jam_blob_encoder_version != self.manifest.jam_blob_encoder_version
            || (!self.manifest.scriptc_revision.is_empty()
                && internal.scriptc_revision != self.manifest.scriptc_revision)
        {
            bail!("toolchain internal manifest does not match distribution manifest");
        }
        if internal.files.is_empty() {
            bail!("toolchain internal manifest has no file hashes");
        }
        for (name, expected) in &internal.files {
            validate_relative_path(name)?;
            let actual = sha256_file(&root.join(name))?;
            if normalize_hash(expected) != actual {
                bail!("toolchain file hash mismatch for `{name}`");
            }
        }
        let installed = self.installed(root, bundle);
        for required in [
            &installed.node,
            &installed.clang,
            &installed.llvm_ar,
            &installed.lld,
            &installed.rustc,
            &installed.cargo,
            &root.join("bin/ar"),
        ] {
            if !required.is_file() {
                bail!(
                    "managed toolchain binary is missing: {}",
                    required.display()
                );
            }
        }
        for required in [&installed.scriptc, &installed.jam_target] {
            if !required.is_dir() {
                bail!(
                    "managed toolchain directory is missing: {}",
                    required.display()
                );
            }
        }
        for required in [&installed.runtime, &installed.runtime_scriptc] {
            if !required.is_dir() {
                bail!(
                    "managed toolchain directory is missing: {}",
                    required.display()
                );
            }
        }
        for required in [&root.join("Cargo.lock"), &installed.polkavm_lock] {
            if !required.is_file() {
                bail!("managed toolchain lock is missing: {}", required.display());
            }
        }
        Ok(())
    }
}

pub fn current_platform() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-x86_64".into()),
        ("linux", "aarch64") => Ok("linux-aarch64".into()),
        ("windows", "x86_64") => Ok("windows-x86_64".into()),
        ("macos", "x86_64") => Ok("macos-x86_64".into()),
        ("macos", "aarch64") => Ok("macos-aarch64".into()),
        _ => bail!("unsupported host platform: {os}-{arch}"),
    }
}

fn cache_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("JAMSCRIPT_TOOLCHAIN_HOME") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("JamScript/toolchains"));
    }
    #[cfg(target_os = "macos")]
    if let Some(path) = env::var_os("HOME") {
        return Ok(PathBuf::from(path).join("Library/Caches/JamScript/toolchains"));
    }
    if let Some(path) = env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(path).join("jamscript/toolchains"));
    }
    let home = env::var_os("HOME").ok_or_else(|| {
        anyhow::anyhow!("cannot determine home directory for JamScript toolchain cache")
    })?;
    Ok(PathBuf::from(home).join(".cache/jamscript/toolchains"))
}

fn offline() -> bool {
    matches!(
        env::var("JAMSCRIPT_OFFLINE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn validate_manifest(manifest: &DistributionManifest) -> Result<()> {
    if manifest.format != 1
        || manifest.toolchain_id.is_empty()
        || manifest.language != "0.2"
        || manifest.backend != "scriptc-m2"
    {
        bail!("unsupported JamScript toolchain distribution manifest");
    }
    if manifest.platforms.is_empty() {
        bail!("toolchain distribution has no platform bundles");
    }
    for bundle in manifest.platforms.values() {
        if bundle.url.is_empty()
            || bundle.sha256.len() != 64
            || !bundle.sha256.chars().all(|c| c.is_ascii_hexdigit())
            || bundle.size == 0
            || (bundle.published && bundle.sha256.chars().all(|c| c == '0'))
        {
            bail!("toolchain distribution contains an incomplete platform bundle");
        }
    }
    Ok(())
}

fn validate_source_locks(manifest: &DistributionManifest, source_root: &Path) -> Result<()> {
    // A released CLI embeds the manifest and does not ship the JamScript
    // source checkout. Source-lock validation runs when the repository is
    // present (contributors and CI); installed CLIs validate the immutable
    // internal bundle manifest instead.
    if !source_root.join("toolchains").is_dir() {
        return Ok(());
    }
    let node = fs::read_to_string(source_root.join("toolchains/scriptc/NODE_VERSION"))?
        .trim()
        .to_string();
    if manifest.node_version != node {
        bail!("TOOLCHAIN_MANIFEST_DRIFT=FAIL: Node version differs from toolchains/scriptc/NODE_VERSION");
    }
    if !manifest.scriptc_revision.is_empty() {
        let revision = fs::read_to_string(source_root.join("toolchains/scriptc/REVISION"))?;
        if !revision.contains(&format!("commit={}", manifest.scriptc_revision)) {
            bail!("TOOLCHAIN_MANIFEST_DRIFT=FAIL: ScriptC revision differs from toolchains/scriptc/REVISION");
        }
    }
    let rust = fs::read_to_string(source_root.join("rust-toolchain.toml"))?;
    if !rust.contains(&format!("channel = \"{}\"", manifest.rust_toolchain)) {
        bail!("TOOLCHAIN_MANIFEST_DRIFT=FAIL: Rust toolchain differs from rust-toolchain.toml");
    }
    let polkavm = fs::read_to_string(source_root.join("toolchains/polkavm.lock"))?;
    if !polkavm.contains(&format!("polkavm_linker = \"{}\"", manifest.polkavm_linker)) {
        bail!("TOOLCHAIN_MANIFEST_DRIFT=FAIL: PolkaVM linker differs from toolchains/polkavm.lock");
    }
    if !polkavm.contains(&format!("clang_version = \"{}\"", manifest.clang_version)) {
        bail!("TOOLCHAIN_MANIFEST_DRIFT=FAIL: Clang version differs from toolchains/polkavm.lock");
    }
    let cargo_lock = fs::read_to_string(source_root.join("Cargo.lock"))?;
    if !cargo_lock.contains(&format!(
        "name = \"polkavm-linker\"\nversion = \"{}\"",
        manifest.polkavm_linker
    )) {
        bail!(
            "TOOLCHAIN_MANIFEST_DRIFT=FAIL: Cargo.lock does not resolve the pinned PolkaVM linker"
        );
    }
    if !cargo_lock.contains(&format!(
        "name = \"jam-program-blob-common\"\nversion = \"{}\"",
        manifest.jam_blob_encoder_version
    )) {
        bail!(
            "TOOLCHAIN_MANIFEST_DRIFT=FAIL: Cargo.lock does not resolve the pinned JAM blob encoder"
        );
    }
    if manifest.jam_target_version != "jam-v1" || manifest.jam_blob_encoder_version != "0.1.28" {
        bail!("TOOLCHAIN_MANIFEST_DRIFT=FAIL: JAM target lock identity is unsupported");
    }
    Ok(())
}

fn download(url: &str, destination: &Path) -> Result<()> {
    if let Some(path) = url.strip_prefix("file://") {
        fs::copy(path, destination).with_context(|| format!("copying toolchain bundle {path}"))?;
        return Ok(());
    }
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--retry",
            "3",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .status()
        .context("downloading toolchain bundle (curl is required only during installation)")?;
    if !status.success() {
        bail!("toolchain download failed: {url}");
    }
    Ok(())
}

fn verify_archive(path: &Path, bundle: &PlatformBundle) -> Result<()> {
    let size = fs::metadata(path)?.len();
    if size != bundle.size {
        bail!(
            "TOOLCHAIN_DOWNLOAD_INTEGRITY=FAIL: expected {} bytes, got {}",
            bundle.size,
            size
        );
    }
    let actual = sha256_file(path)?;
    if actual != normalize_hash(&bundle.sha256) {
        bail!("TOOLCHAIN_DOWNLOAD_INTEGRITY=FAIL: SHA-256 mismatch");
    }
    Ok(())
}

fn extract_archive(archive: &Path, destination: &Path, kind: &str) -> Result<()> {
    let mut listing = tar::Archive::new(archive_reader(archive, kind)?);
    let mut total_size = 0u64;
    for entry in listing.entries().context("listing toolchain archive")? {
        let entry = entry.context("reading toolchain archive entry")?;
        validate_relative_path(
            &entry
                .path()
                .context("reading toolchain archive path")?
                .to_string_lossy(),
        )?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            bail!("toolchain archive contains symlinks or hard links");
        }
        total_size = total_size.saturating_add(entry.header().size().unwrap_or(0));
        if total_size > 8 * 1024 * 1024 * 1024 {
            bail!("toolchain archive exceeds the 8 GiB extraction limit");
        }
    }
    let mut unpack = tar::Archive::new(archive_reader(archive, kind)?);
    unpack
        .unpack(destination)
        .context("extracting toolchain archive")?;
    Ok(())
}

fn archive_reader(archive: &Path, kind: &str) -> Result<Box<dyn Read>> {
    let file = File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    if kind.ends_with(".zst") {
        Ok(Box::new(
            zstd::Decoder::new(file).context("opening zstd toolchain archive")?,
        ))
    } else {
        Ok(Box::new(file))
    }
}

fn normalize_archive_root(staging: &Path) -> Result<PathBuf> {
    if staging.join("manifest.json").is_file() {
        return Ok(staging.to_path_buf());
    }
    let mut entries = fs::read_dir(staging)?.collect::<std::result::Result<Vec<_>, _>>()?;
    if entries.len() == 1
        && entries[0].path().is_dir()
        && entries[0].path().join("manifest.json").is_file()
    {
        return Ok(entries.remove(0).path());
    }
    bail!("toolchain archive must contain manifest.json at its root")
}

fn validate_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || value.contains('\\')
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("unsafe toolchain archive path `{value}`");
    }
    Ok(())
}

fn normalize_hash(value: &str) -> String {
    value.trim().trim_start_matches("0x").to_ascii_lowercase()
}

struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    fn acquire(cache: &Path) -> Result<Self> {
        let path = cache.join(".install.lock");
        for _ in 0..1200 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    writeln!(file, "{}", std::process::id())?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", path.display()))
                }
            }
        }
        bail!(
            "timed out waiting for JamScript toolchain installation lock {}",
            path.display()
        )
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 64];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_names_are_stable() {
        assert!(matches!(
            current_platform().unwrap().as_str(),
            "linux-x86_64" | "linux-aarch64" | "windows-x86_64" | "macos-x86_64" | "macos-aarch64"
        ));
    }

    #[test]
    fn archive_paths_are_rejected_before_extraction() {
        assert!(validate_relative_path("../escape").is_err());
        assert!(validate_relative_path("/absolute").is_err());
        assert!(validate_relative_path("tool\\escape").is_err());
        assert!(validate_relative_path("bin/clang").is_ok());
    }

    #[test]
    fn manifest_matches_repository_locks() {
        ToolchainManager::new().unwrap();
    }
}
