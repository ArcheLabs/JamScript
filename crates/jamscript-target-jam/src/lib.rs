use anyhow::{bail, Context, Result};
use jam_program_blob_common::ProgramBlob;
use jamscript_backend_scriptc::{ScriptcArtifact, ScriptcBuildMetadata, ScriptcCompiler};
use jamscript_codegen_rust::{
    generate_builder_application_rust, generate_no_std_rust_with_scriptc_context,
    ManagementPolicyConfig, PortableServiceContext,
};
use jamscript_ir::{abi_for_language, ServiceIr, NATIVE_ABI_VERSION};
use jamscript_toolchain::InstalledToolchain;
use serde::{Deserialize, Serialize};
use service_build_polkavm::{
    GuestBuildArtifacts, NativeArchive, PolkaVmBuildConfig, PolkaVmBuildRequest, PolkaVmBuilder,
};
use service_runtime_core::{
    MANAGED_STATE_LAYOUT_VERSION, MANAGED_STATE_PROTOCOL_VERSION, RECOVERY_FORMAT_VERSION,
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::tempdir;

#[derive(Clone, Debug, Serialize)]
pub struct BuildMetadata {
    pub jamscript_toolchain_id: String,
    pub jamscript_toolchain_platform: String,
    pub jamscript_toolchain_sha256: String,
    pub canonical_toolchain: bool,
    #[serde(rename = "serviceKey")]
    pub service_key: String,
    #[serde(rename = "serviceInstanceId")]
    pub service_instance_id: String,
    pub management: ManagementMetadata,
    pub language_version: String,
    pub compiler_version: String,
    pub runtime_version: String,
    #[serde(rename = "runtimePackageVersion")]
    pub runtime_package_version: String,
    #[serde(rename = "managedStateProtocolVersion")]
    pub managed_state_protocol_version: u8,
    #[serde(rename = "managedStateLayoutVersion")]
    pub managed_state_layout_version: u8,
    #[serde(rename = "runtimeRefineInputVersion")]
    pub runtime_refine_input_version: u8,
    #[serde(rename = "signedActionVersion")]
    pub signed_action_version: u8,
    #[serde(rename = "recoveryFormatVersion")]
    pub recovery_format_version: u8,
    pub abi_version: u32,
    pub jam_target_version: String,
    #[serde(rename = "jamBlobEncoder")]
    pub jam_blob_encoder: String,
    #[serde(rename = "jamBlobEncoderVersion")]
    pub jam_blob_encoder_version: String,
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
    pub source_hash: String,
    pub abi_hash: String,
    pub code_hash: Option<String>,
    pub native_abi_version: u32,
    pub native_modules: Vec<NativeModuleMetadata>,
    #[serde(flatten)]
    pub scriptc: Option<ScriptcBuildMetadata>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ManagementMetadata {
    pub mode: String,
    #[serde(rename = "genesisAccount", skip_serializing_if = "Option::is_none")]
    pub genesis_account: Option<String>,
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

#[derive(Clone, Debug, Serialize)]
pub struct BuilderArtifactMetadata {
    pub version: u8,
    pub application: String,
    pub native_modules: Vec<BuilderNativeModuleMetadata>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BuilderNativeModuleMetadata {
    pub name: String,
    pub sources: Vec<String>,
    pub include_dirs: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ProtocolBoundaryV0 {
    release_channel: &'static str,
    language: &'static str,
    signed_action: &'static str,
    #[serde(rename = "signedActionVersion")]
    signed_action_version: u8,
    application_abi: u8,
    managed_state_protocol: u8,
    managed_state_layout: u8,
    #[serde(rename = "runtimeRefineInputVersion")]
    runtime_refine_input: u8,
    recovery_format: u8,
    builder_artifact: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BundleChecksums {
    version: u8,
    algorithm: String,
    files: BTreeMap<String, String>,
}

pub fn verify_deployment_bundle(bundle: &Path) -> Result<BTreeMap<String, String>> {
    let manifest_path = bundle.join("checksums.json");
    let manifest: BundleChecksums = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )?;
    if manifest.version != 1 || manifest.algorithm != "blake2b-256" {
        bail!("unsupported deployment bundle checksum manifest");
    }
    for (name, expected) in &manifest.files {
        if Path::new(name).is_absolute() || name.contains("..") {
            bail!("invalid deployment bundle path `{name}`");
        }
        let actual = hash_file(&bundle.join(name))?;
        if &actual != expected {
            bail!("deployment bundle checksum mismatch for `{name}`");
        }
    }
    Ok(manifest.files)
}

pub struct JamTarget {
    pub sdk_root: PathBuf,
    pub toolchain: Option<InstalledToolchain>,
}

impl JamTarget {
    pub fn new() -> Self {
        let sdk_root = workspace_root().join("crates/jamscript-target-jam/sdk");
        Self {
            sdk_root,
            toolchain: None,
        }
    }

    pub fn from_installed_toolchain(toolchain: &InstalledToolchain) -> Self {
        Self {
            sdk_root: toolchain.jam_target.clone(),
            toolchain: Some(toolchain.clone()),
        }
    }

    pub fn target_sdk_root(&self) -> &Path {
        &self.sdk_root
    }

    pub fn build_scriptc_probe(
        &self,
        project_root: &Path,
        ir: &ServiceIr,
        context: PortableServiceContext,
        output_dir: &Path,
        native_modules: &[NativeModule],
    ) -> Result<BuildMetadata> {
        let scriptc_root = self
            .toolchain
            .as_ref()
            .map(|toolchain| toolchain.scriptc.clone())
            .unwrap_or_else(|| workspace_root().join("toolchains/scriptc"));
        let compiler = match self.toolchain.as_ref() {
            Some(toolchain) => ScriptcCompiler::from_paths(&scriptc_root, &toolchain.node)?,
            None => ScriptcCompiler::from_toolchain(&scriptc_root)?,
        };
        let artifact = compiler.compile_service(ir, &output_dir.join("scriptc"))?;
        self.build_probe_inner(
            project_root,
            ir,
            context,
            output_dir,
            native_modules,
            artifact,
        )
    }

    fn build_probe_inner(
        &self,
        project_root: &Path,
        ir: &ServiceIr,
        context: PortableServiceContext,
        output_dir: &Path,
        native_modules: &[NativeModule],
        scriptc: ScriptcArtifact,
    ) -> Result<BuildMetadata> {
        fs::create_dir_all(output_dir)?;
        let generated = output_dir.join("generated_service.rs");
        let generated_source = generate_no_std_rust_with_scriptc_context(ir, context)
            .map_err(|error| anyhow::anyhow!(error))?;
        fs::write(&generated, generated_source)
            .with_context(|| format!("writing {}", generated.display()))?;
        fs::write(
            output_dir.join("generated_builder_application.rs"),
            generate_builder_application_rust(ir, context)
                .map_err(|error| anyhow::anyhow!(error))?,
        )?;
        fs::write(
            output_dir.join("builder.json"),
            serde_json::to_vec_pretty(&builder_metadata(project_root, native_modules))?,
        )?;
        fs::write(
            output_dir.join("protocol-v0.json"),
            serde_json::to_vec_pretty(&ProtocolBoundaryV0 {
                release_channel: "testnet-developer-preview",
                language: "0.2",
                signed_action: "SignedActionV1",
                signed_action_version: 1,
                application_abi: 1,
                managed_state_protocol: MANAGED_STATE_PROTOCOL_VERSION,
                managed_state_layout: MANAGED_STATE_LAYOUT_VERSION,
                runtime_refine_input: 1,
                recovery_format: RECOVERY_FORMAT_VERSION,
                builder_artifact: 1,
            })?,
        )?;
        let abi = abi_for_language(ir, "0.2")?;
        fs::write(
            output_dir.join("service.abi.json"),
            serde_json::to_vec_pretty(&abi)?,
        )?;
        let source_hash = hash_file(&generated)?;
        let abi_hash = hash_file(&output_dir.join("service.abi.json"))?;

        let guest_project = tempdir().context("creating Rust guest project")?;
        fs::create_dir_all(guest_project.path().join("src"))?;
        let runtime_root = self
            .toolchain
            .as_ref()
            .map(|toolchain| toolchain.runtime.clone());
        let runtime_crate = |name: &str| -> Result<PathBuf> {
            match &runtime_root {
                Some(root) => root
                    .join("crates")
                    .join(name)
                    .canonicalize()
                    .with_context(|| format!("locating managed runtime crate {name}")),
                None => workspace_crate(name),
            }
        };
        let runtime_core = runtime_crate("jamscript-runtime-core")?;
        let service_runtime_core = runtime_crate("service-runtime-core")?;
        let service_runtime_guest = runtime_crate("service-runtime-guest")?;
        let diagnostic_feature = if context.diagnostic {
            ", features = [\"diagnostic\"]"
        } else {
            ""
        };
        fs::write(guest_project.path().join("Cargo.toml"), format!(
            "[package]\nname = \"jamscript_guest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\ncrate-type = [\"cdylib\"]\n[dependencies]\njamscript-runtime-core = {{ path = \"{}\", default-features = false }}\nservice-runtime-core = {{ path = \"{}\", default-features = false }}\nservice-runtime-guest = {{ path = \"{}\", default-features = false{} }}\n[workspace]\nresolver = \"2\"\n",
            runtime_core.display(), service_runtime_core.display(), service_runtime_guest.display(), diagnostic_feature
        ))?;
        fs::copy(&generated, guest_project.path().join("src/lib.rs"))?;
        fs::write(guest_project.path().join("build.rs"), build_script())?;

        let work = tempdir().context("creating JAM target native build directory")?;
        let clang = pinned_clang(self.toolchain.as_ref())?;
        let ar = self
            .toolchain
            .as_ref()
            .map(|toolchain| toolchain.llvm_ar.as_path());
        let mut archives = vec![compile_jam_archive(
            &self.sdk_root,
            &clang,
            ar,
            work.path(),
        )?];
        let scriptc_root = self
            .toolchain
            .as_ref()
            .map(|toolchain| toolchain.scriptc.clone())
            .unwrap_or_else(|| workspace_root().join("toolchains/scriptc"));
        archives.push(compile_scriptc_archive(
            &scriptc,
            &scriptc_root,
            self.toolchain
                .as_ref()
                .map(|toolchain| toolchain.runtime_scriptc.as_path()),
            &clang,
            ar,
            work.path(),
        )?);
        for module in native_modules {
            archives.push(compile_native_archive(module, &clang, ar, work.path())?);
        }
        let backend_output = work.path().join("polkavm");
        let artifacts = PolkaVmBuilder::new(PolkaVmBuildConfig {
            rust_toolchain: self
                .toolchain
                .as_ref()
                .map(|toolchain| toolchain.rust_toolchain.clone())
                .unwrap_or_else(|| "nightly-2026-05-02".into()),
            diagnostic: context.diagnostic,
            rustflags: Some(jam_rustflags()),
            cargo_path: self
                .toolchain
                .as_ref()
                .map(|toolchain| toolchain.cargo.clone()),
            rustc_path: self
                .toolchain
                .as_ref()
                .map(|toolchain| toolchain.rustc.clone()),
            cargo_home: self
                .toolchain
                .as_ref()
                .map(|toolchain| toolchain.cargo_home.clone()),
            lock_path: self
                .toolchain
                .as_ref()
                .map(|toolchain| toolchain.polkavm_lock.clone())
                .unwrap_or_else(|| workspace_root().join("toolchains/polkavm.lock")),
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
        link_elf_to_jam(&artifacts.elf, &blob, &polkavm)?;
        fs::copy(&polkavm, output_dir.join("service.pvm"))?;
        let mut checksum_files = vec![
            "service.blob",
            "service.polkavm",
            "service.pvm",
            "service.elf",
            "service.abi.json",
            "generated_service.rs",
            "generated_builder_application.rs",
            "builder.json",
            "protocol-v0.json",
        ];
        checksum_files.extend([
            "scriptc/scriptc_service.ts",
            "scriptc/scriptc_service.json",
            "scriptc/scriptc_service.transformed.ts",
            "scriptc/scriptc_runtime.ts",
            "scriptc/scriptc_service.profile.json",
            "scriptc/scriptc_service.lib.c",
            "scriptc/scriptc_service_adapter.c",
        ]);
        let files = checksum_files
            .into_iter()
            .map(|name| Ok((name.to_owned(), hash_file(&output_dir.join(name))?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        fs::write(
            output_dir.join("checksums.json"),
            serde_json::to_vec_pretty(&BundleChecksums {
                version: 1,
                algorithm: "blake2b-256".into(),
                files,
            })?,
        )?;
        let clang_version = command_version(&clang)?;
        let native_metadata = native_metadata(project_root, native_modules)?;
        Ok(build_metadata(
            context,
            source_hash,
            abi_hash,
            hash_file(&blob)?,
            clang_version,
            native_metadata,
            artifacts,
            Some(scriptc.metadata.clone()),
            "0.2",
            self.toolchain.as_ref(),
        ))
    }
}

impl Default for JamTarget {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a RISC-V guest ELF into a PolkaVM debug artifact and canonical JAM
/// ProgramBlob using the locked JamV1 conversion semantics.
pub fn elf_to_jam_blob(elf: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut config = polkavm_linker::Config::default();
    config.set_strip(true);
    config.set_dispatch_table(vec![
        b"minijam_refine".to_vec(),
        b"minijam_accumulate".to_vec(),
    ]);
    let linked =
        polkavm_linker::program_from_elf(config, polkavm_linker::TargetInstructionSet::JamV1, elf)
            .map_err(|error| anyhow::anyhow!("link ELF for JAM target: {error}"))?;
    let parts = polkavm_linker::ProgramParts::from_bytes(linked.clone().into())
        .map_err(|error| anyhow::anyhow!("decode linked PolkaVM program: {error}"))?;
    let blob = ProgramBlob::from_pvm(&parts, Cow::Borrowed(&[]))
        .to_vec()
        .map_err(|error| anyhow::anyhow!("materialize JAM ProgramBlob: {error}"))?;
    Ok((linked, blob))
}

pub fn link_elf_to_jam(elf: &Path, blob: &Path, polkavm: &Path) -> Result<()> {
    let input = fs::read(elf).with_context(|| format!("reading {}", elf.display()))?;
    let (linked, encoded_blob) = elf_to_jam_blob(&input)?;
    fs::write(polkavm, linked).with_context(|| format!("writing {}", polkavm.display()))?;
    fs::write(blob, encoded_blob).with_context(|| format!("writing {}", blob.display()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_metadata(
    context: PortableServiceContext,
    source_hash: String,
    abi_hash: String,
    code_hash: String,
    clang_version: String,
    native_modules: Vec<NativeModuleMetadata>,
    artifacts: GuestBuildArtifacts,
    scriptc: Option<ScriptcBuildMetadata>,
    language_version: &str,
    managed_toolchain: Option<&InstalledToolchain>,
) -> BuildMetadata {
    let toolchain = artifacts.metadata;
    BuildMetadata {
        jamscript_toolchain_id: managed_toolchain
            .map(|toolchain| toolchain.toolchain_id.clone())
            .unwrap_or_else(|| "development".into()),
        jamscript_toolchain_platform: managed_toolchain
            .map(|toolchain| toolchain.platform.clone())
            .unwrap_or_else(|| "development".into()),
        jamscript_toolchain_sha256: managed_toolchain
            .map(|toolchain| toolchain.bundle_sha256.clone())
            .unwrap_or_default(),
        canonical_toolchain: managed_toolchain.is_some(),
        service_key: format!(
            "0x{}",
            context
                .service_key
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        service_instance_id: format!(
            "0x{}",
            context
                .service_instance_id
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        management: match context.management_policy {
            ManagementPolicyConfig::Immutable => ManagementMetadata {
                mode: "immutable".into(),
                genesis_account: None,
            },
            ManagementPolicyConfig::Key { account } => ManagementMetadata {
                mode: "key".into(),
                genesis_account: Some(format!(
                    "0x{}",
                    account
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                )),
            },
        },
        language_version: language_version.into(),
        compiler_version: env!("CARGO_PKG_VERSION").into(),
        runtime_version: "0.1.0".into(),
        runtime_package_version: "service-runtime-0.1.0".into(),
        managed_state_protocol_version: MANAGED_STATE_PROTOCOL_VERSION,
        managed_state_layout_version: MANAGED_STATE_LAYOUT_VERSION,
        runtime_refine_input_version: 1,
        signed_action_version: 1,
        recovery_format_version: RECOVERY_FORMAT_VERSION,
        abi_version: 1,
        jam_target_version: "jam-v1".into(),
        jam_blob_encoder: "jam-program-blob-common".into(),
        jam_blob_encoder_version: "0.1.28".into(),
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
        source_hash,
        abi_hash,
        code_hash: Some(code_hash),
        native_abi_version: NATIVE_ABI_VERSION,
        native_modules,
        scriptc,
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
    use super::{hash_file, jam_rustflags_from, verify_deployment_bundle, BundleChecksums};
    use std::{collections::BTreeMap, fs};

    #[test]
    fn jam_metadata_relocation_override_is_explicit() {
        assert_eq!(jam_rustflags_from(""), "-C link-arg=-z -C link-arg=notext");
        assert_eq!(
            jam_rustflags_from("-C opt-level=2"),
            "-C opt-level=2 -C link-arg=-z -C link-arg=notext"
        );
    }

    #[test]
    fn deployment_bundle_verification_rejects_tampering() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("service.blob"), b"canonical").unwrap();
        let manifest = BundleChecksums {
            version: 1,
            algorithm: "blake2b-256".into(),
            files: BTreeMap::from([(
                "service.blob".into(),
                hash_file(&temp.path().join("service.blob")).unwrap(),
            )]),
        };
        fs::write(
            temp.path().join("checksums.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_deployment_bundle(temp.path()).is_ok());
        fs::write(temp.path().join("service.blob"), b"tampered").unwrap();
        assert!(verify_deployment_bundle(temp.path()).is_err());
    }
}

fn workspace_crate(name: &str) -> Result<PathBuf> {
    workspace_root()
        .join("crates")
        .join(name)
        .canonicalize()
        .with_context(|| format!("locating workspace crate {name}"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
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

fn pinned_clang(toolchain: Option<&InstalledToolchain>) -> Result<PathBuf> {
    let clang = match toolchain {
        Some(toolchain) => toolchain.clang.clone(),
        None => std::env::var_os("JAMSCRIPT_CLANG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/clang")),
    };
    if !clang.is_file() {
        bail!(
            "pinned Clang 20 was not found at {}; set JAMSCRIPT_CLANG",
            clang.display()
        );
    }
    Ok(clang)
}

fn compile_jam_archive(
    sdk_root: &Path,
    clang: &Path,
    ar: Option<&Path>,
    work: &Path,
) -> Result<NativeArchive> {
    let include = sdk_root.join("include");
    let source_root = sdk_root.join("src");
    let sources = ["host", "minijam", "crypto"].map(|unit| source_root.join(format!("{unit}.c")));
    compile_archive("jam_target_guest", &sources, &[include], clang, ar, work)
}

fn compile_native_archive(
    module: &NativeModule,
    clang: &Path,
    ar: Option<&Path>,
    work: &Path,
) -> Result<NativeArchive> {
    compile_archive(
        &format!("native_{}", module.name),
        &module.sources,
        &module.include_dirs,
        clang,
        ar,
        work,
    )
}

fn compile_scriptc_archive(
    artifact: &ScriptcArtifact,
    toolchain_root: &Path,
    managed_runtime_root: Option<&Path>,
    clang: &Path,
    ar: Option<&Path>,
    work: &Path,
) -> Result<NativeArchive> {
    let runtime = toolchain_root.join("node_modules/@scriptc/runtime/src");
    if !runtime.is_dir() {
        bail!("ScriptC runtime is not installed at {}", runtime.display());
    }
    let runtime_include = managed_runtime_root
        .map(|root| root.join("include"))
        .unwrap_or_else(|| workspace_root().join("crates/jamscript-runtime-scriptc/include"));
    let mut sources = vec![artifact.generated_c.clone(), artifact.adapter_c.clone()];
    sources.extend(
        jamscript_runtime_scriptc::selected_runtime_units()
            .iter()
            .map(|name| {
                if *name == "scr_lib_cleanup.c" {
                    managed_runtime_root
                        .map(|root| root.join("src/scr_lib_cleanup.c"))
                        .unwrap_or_else(|| {
                            workspace_root()
                                .join("crates/jamscript-runtime-scriptc/src/scr_lib_cleanup.c")
                        })
                } else if *name == "freestanding.c" {
                    managed_runtime_root
                        .map(|root| root.join("src/freestanding.c"))
                        .unwrap_or_else(|| {
                            workspace_root()
                                .join("crates/jamscript-runtime-scriptc/src/freestanding.c")
                        })
                } else {
                    runtime.join(name)
                }
            }),
    );
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
        "-DSCR_LIB",
    ];
    let mut objects = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let object = work.join(format!("scriptc_runtime_{index}.o"));
        let mut command = Command::new(clang);
        command.args(common).arg("-std=c11");
        command.arg("-I").arg(&runtime_include);
        command.arg("-I").arg(&runtime);
        command.args([
            "-c",
            source.to_str().unwrap(),
            "-o",
            object.to_str().unwrap(),
        ]);
        run(
            &mut command,
            &format!("compiling ScriptC runtime {}", source.display()),
        )?;
        objects.push(object);
    }
    let ar = match ar.map(Path::to_path_buf) {
        Some(path) if path.is_file() => path,
        Some(path) => bail!("managed llvm-ar is missing at {}", path.display()),
        None => std::env::var_os("JAMSCRIPT_LLVM_AR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/llvm-ar")),
    };
    let archive = work.join("libscriptc_runtime.a");
    let mut command = Command::new(ar);
    command.arg("crs").arg(&archive).args(&objects);
    run(&mut command, "archiving ScriptC runtime")?;
    Ok(NativeArchive {
        name: "scriptc_runtime".into(),
        path: archive,
    })
}

fn compile_archive(
    name: &str,
    sources: &[PathBuf],
    include_dirs: &[PathBuf],
    clang: &Path,
    ar: Option<&Path>,
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
    let ar = match ar.map(Path::to_path_buf) {
        Some(path) if path.is_file() => path,
        Some(path) => bail!("managed llvm-ar is missing at {}", path.display()),
        None => {
            let path = std::env::var_os("JAMSCRIPT_LLVM_AR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/llvm-ar"));
            if path.is_file() {
                path
            } else {
                PathBuf::from("ar")
            }
        }
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

fn builder_metadata(project_root: &Path, modules: &[NativeModule]) -> BuilderArtifactMetadata {
    let relative = |path: &Path| {
        path.strip_prefix(project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    BuilderArtifactMetadata {
        version: 1,
        application: "generated_builder_application.rs".into(),
        native_modules: modules
            .iter()
            .map(|module| BuilderNativeModuleMetadata {
                name: module.name.clone(),
                sources: module.sources.iter().map(|path| relative(path)).collect(),
                include_dirs: module
                    .include_dirs
                    .iter()
                    .map(|path| relative(path))
                    .collect(),
            })
            .collect(),
    }
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
