use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use jamscript_codegen_rust::{
    generate_builder_application_rust, ManagementPolicyConfig, PortableServiceContext,
};
use jamscript_ir::abi_for_language;
use jamscript_parser::parse_service_v02;
use jamscript_target_minijam::{verify_deployment_bundle, MiniJamTarget, NativeModule};
use serde::Deserialize;
use service_runtime_core::ServiceKeyV1;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "jamscript",
    version,
    about = "Deterministic TypeScript-like JAM Service toolchain"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    New {
        name: String,
    },
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Abi {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Build {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "dist")]
        output: PathBuf,
    },
    Inspect {
        #[arg(default_value = "dist")]
        bundle: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    package: Package,
    compiler: Option<CompilerConfig>,
    target: Option<Target>,
    native: Option<BTreeMap<String, NativeConfig>>,
    management: Option<ManagementConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompilerConfig {
    backend: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceMetadata {
    version: u8,
    #[serde(rename = "serviceKey")]
    service_key: String,
    #[serde(rename = "instanceId")]
    instance_id: Option<String>,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagementConfig {
    #[serde(default = "default_management_mode")]
    mode: String,
    account: Option<String>,
}

fn default_management_mode() -> String {
    "deployer".into()
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    name: String,
    version: String,
    entry: String,
    language: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    minijam: Option<MiniJamConfig>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MiniJamConfig {
    sdk_root: Option<String>,
    /// Legacy deployment/routing identifier. It is never embedded in
    /// SignedActionV1 or generated Service identity.
    #[allow(dead_code)]
    service_id: Option<u32>,
    /// Legacy manifest spelling for the network domain (genesis hash).
    genesis_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeConfig {
    language: String,
    sources: Vec<String>,
    #[serde(default)]
    include_dirs: Vec<String>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        CommandKind::New { name } => new_project(&name),
        CommandKind::Check { path } => {
            let (_, ir) = load(&path)?;
            let actions = ir
                .actions
                .iter()
                .map(|action| action.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("checked {} actions [{}]", path.display(), actions);
            Ok(())
        }
        CommandKind::Abi { path } => {
            let (_manifest, ir) = load(&path)?;
            let abi = abi_for_language(&ir, "0.2")?;
            println!("{}", serde_json::to_string_pretty(&abi)?);
            Ok(())
        }
        CommandKind::Build { path, output } => build(&path, &output),
        CommandKind::Inspect { bundle } => inspect(&bundle),
    }
}

fn inspect(bundle: &Path) -> Result<()> {
    let files = verify_deployment_bundle(bundle)?;
    let read_json = |name: &str| -> Result<serde_json::Value> {
        let path = bundle.join(name);
        serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("decoding {}", path.display()))
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "verified": true,
            "files": files.len(),
            "build": read_json("build.json")?,
            "protocol": read_json("protocol-v0.json")?,
            "builder": read_json("builder.json")?,
        }))?
    );
    Ok(())
}

fn load(path: &Path) -> Result<(Manifest, jamscript_ir::ServiceIr)> {
    let manifest_path = path.join("jamscript.toml");
    let manifest: Manifest = toml::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )?;
    if let Some(management) = &manifest.management {
        match management.mode.as_str() {
            "deployer" => {
                if let Some(account) = management.account.as_deref() {
                    parse_hash(account).with_context(|| {
                        "[management] deployer account must be a 32-byte hex key"
                    })?;
                }
            }
            "immutable" => {
                if management.account.is_some() {
                    bail!("[management] account is not valid with mode = \"immutable\"");
                }
            }
            "key" => {
                let account = management
                    .account
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("[management] key mode requires account"))?;
                parse_hash(account)
                    .with_context(|| "[management] account must be a 32-byte hex key")?;
            }
            mode => bail!("unsupported management mode `{mode}`"),
        }
    }
    let backend = manifest
        .compiler
        .as_ref()
        .map(|config| config.backend.as_str());
    if manifest.package.language != "0.2" {
        bail!(
            "unsupported JamScript language version {}; supported version: 0.2",
            manifest.package.language
        );
    }
    if backend != Some("scriptc") {
        bail!("language 0.2 requires [compiler] backend = \"scriptc\"");
    }
    let source_path = path.join(&manifest.package.entry);
    let source = fs::read_to_string(&source_path)
        .with_context(|| format!("reading {}", source_path.display()))?;
    let native_modules = manifest
        .native
        .as_ref()
        .map(|modules| modules.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let ir = parse_service_v02(
        &source,
        &manifest.package.name,
        &manifest.package.version,
        &native_modules,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", source_path.display()))?;
    Ok((manifest, ir))
}

fn load_service_identity(path: &Path) -> Result<(ServiceKeyV1, [u8; 32])> {
    let metadata_path = path.join(".jamscript/service.json");
    let metadata: ServiceMetadata = serde_json::from_str(
        &fs::read_to_string(&metadata_path)
            .with_context(|| format!("reading {}", metadata_path.display()))?,
    )?;
    if (metadata.version != 1 && metadata.version != 2) || metadata.name.is_empty() {
        bail!("invalid service metadata in {}", metadata_path.display());
    }
    let bytes = parse_hash(&metadata.service_key)?;
    let instance_id = match metadata.instance_id {
        Some(value) => parse_hash(&value)?,
        None => {
            let mut generated = [0u8; 32];
            getrandom::fill(&mut generated)
                .map_err(|error| anyhow::anyhow!("generating service instance id: {error:?}"))?;
            let updated = serde_json::json!({
                "version": 2,
                "serviceKey": format!("0x{}", encode_hex(&bytes)),
                "instanceId": format!("0x{}", encode_hex(&generated)),
                "name": metadata.name,
            });
            fs::write(metadata_path, serde_json::to_vec_pretty(&updated)?)?;
            generated
        }
    };
    Ok((ServiceKeyV1::new(bytes), instance_id))
}

fn new_project(name: &str) -> Result<()> {
    let root = PathBuf::from(name);
    if root.exists() {
        bail!("directory already exists: {}", root.display());
    }
    fs::create_dir_all(root.join("src"))?;
    fs::create_dir_all(root.join(".jamscript"))?;
    let mut service_key = [0u8; 32];
    getrandom::fill(&mut service_key)
        .map_err(|error| anyhow::anyhow!("generating service key: {error:?}"))?;
    let mut instance_id = [0u8; 32];
    getrandom::fill(&mut instance_id)
        .map_err(|error| anyhow::anyhow!("generating service instance id: {error:?}"))?;
    let service_key = service_key
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(root.join("jamscript.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.2.0\"\nentry = \"src/service.ts\"\nlanguage = \"0.2\"\n\n[compiler]\nbackend = \"scriptc\"\n\n[management]\nmode = \"deployer\"\n"))?;
    fs::write(root.join("src/service.ts"), "import { action, wallet, u64 } from \"jam\";\n\nexport const increment = action({\n  auth: wallet(),\n  input: { value: u64 },\n  execute(ctx, input) {\n    return input.value + 1;\n  },\n});\n")?;
    fs::write(
        root.join(".jamscript/service.json"),
        format!("{{\n  \"version\": 2,\n  \"serviceKey\": \"0x{service_key}\",\n  \"instanceId\": \"0x{}\",\n  \"name\": \"{name}\"\n}}\n", encode_hex(&instance_id)),
    )?;
    println!("created {}", root.display());
    Ok(())
}

fn build(path: &Path, output: &Path) -> Result<()> {
    let (manifest, ir) = load(path)?;
    let minijam = manifest
        .target
        .as_ref()
        .and_then(|target| target.minijam.as_ref());
    let (service_key, service_instance_id) = load_service_identity(path)?;
    let management_policy = resolve_management_policy(manifest.management.as_ref(), &service_key)?;
    let context = PortableServiceContext {
        service_key: service_key.into_bytes(),
        service_instance_id,
        management_policy,
        genesis_hash: minijam
            .and_then(|target| target.genesis_hash.as_deref())
            .map(parse_hash)
            .transpose()?
            .unwrap_or([0; 32]),
        diagnostic: std::env::var_os("JAMSCRIPT_DIAGNOSTIC_GUEST").is_some(),
    };
    fs::create_dir_all(output)?;
    let abi = abi_for_language(&ir, "0.2")?;
    fs::write(
        output.join("service.abi.json"),
        serde_json::to_vec_pretty(&abi)?,
    )?;
    fs::write(
        output.join("generated_service.rs"),
        jamscript_codegen_rust::generate_no_std_rust_with_scriptc_context(&ir, context)
            .map_err(|e| anyhow::anyhow!(e))?,
    )?;
    fs::write(
        output.join("generated_builder_application.rs"),
        generate_builder_application_rust(&ir, context).map_err(|e| anyhow::anyhow!(e))?,
    )?;
    let sdk_root = minijam
        .and_then(|target| target.sdk_root.as_deref())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("JAMSCRIPT_MINIJAM_SDK").map(PathBuf::from))
        .or_else(|| {
            [
                path.join("../../../minijam-client"),
                path.join("../../minijam-client"),
                std::env::current_dir().ok()?.join("../minijam-client"),
            ]
            .into_iter()
            .find(|candidate| candidate.join("service-toolchain/sdk").is_dir())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "MiniJAM SDK not found; set target.minijam.sdk_root or JAMSCRIPT_MINIJAM_SDK"
            )
        })?;
    let project_root = path
        .canonicalize()
        .with_context(|| format!("canonicalizing project root {}", path.display()))?;
    let native_modules = resolve_native_modules(&project_root, manifest.native.as_ref())?;
    let target = MiniJamTarget::from_sdk_root(sdk_root);
    let metadata = target
        .build_scriptc_probe(&project_root, &ir, context, output, &native_modules)
        .context("MiniJAM target build")?;
    fs::write(
        output.join("build.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!("built {}", output.display());
    Ok(())
}

fn resolve_management_policy(
    config: Option<&ManagementConfig>,
    service_key: &ServiceKeyV1,
) -> Result<ManagementPolicyConfig> {
    let Some(config) = config else {
        return Ok(ManagementPolicyConfig::Immutable);
    };
    match config.mode.as_str() {
        "immutable" => Ok(ManagementPolicyConfig::Immutable),
        "key" => Ok(ManagementPolicyConfig::Key {
            account: parse_hash(
                config
                    .account
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("[management] key mode requires account"))?,
            )?,
        }),
        "deployer" => {
            let account = config
                .account
                .as_deref()
                .map(parse_hash)
                .transpose()?
                .or_else(|| {
                    std::env::var("JAMSCRIPT_DEPLOYER_ACCOUNT")
                        .ok()
                        .and_then(|value| parse_hash(&value).ok())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "[management] deployer requires account or JAMSCRIPT_DEPLOYER_ACCOUNT"
                    )
                })?;
            if account == service_key.into_bytes() {
                bail!("[management] deployer account must be a wallet public key, not serviceKey");
            }
            Ok(ManagementPolicyConfig::Key { account })
        }
        mode => bail!("unsupported management mode `{mode}`"),
    }
}

fn resolve_native_modules(
    root: &Path,
    configs: Option<&BTreeMap<String, NativeConfig>>,
) -> Result<Vec<NativeModule>> {
    let Some(configs) = configs else {
        return Ok(Vec::new());
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalizing project root {}", root.display()))?;
    configs
        .iter()
        .map(|(name, config)| {
            if config.language != "c" {
                bail!("native module `{name}` must use language = \"c\"");
            }
            if config.sources.is_empty() {
                bail!("native module `{name}` must declare at least one source");
            }
            let sources = config
                .sources
                .iter()
                .map(|value| resolve_project_path(&root, value, "native source"))
                .collect::<Result<Vec<_>>>()?;
            let include_dirs = config
                .include_dirs
                .iter()
                .map(|value| resolve_project_path(&root, value, "native include directory"))
                .collect::<Result<Vec<_>>>()?;
            Ok(NativeModule {
                name: name.clone(),
                sources,
                include_dirs,
            })
        })
        .collect()
}

fn resolve_project_path(root: &Path, value: &str, kind: &str) -> Result<PathBuf> {
    let relative = Path::new(value);
    if relative.is_absolute() {
        bail!("{kind} `{value}` must be relative to the project root");
    }
    let path = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("resolving {kind} `{value}`"))?;
    if !path.starts_with(root) {
        bail!("{kind} `{value}` escapes the project root");
    }
    Ok(path)
}

fn parse_hash(value: &str) -> Result<[u8; 32]> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() != 64 {
        bail!("genesis_hash must contain exactly 32 bytes of hexadecimal data");
    }
    let mut hash = [0u8; 32];
    for (index, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .with_context(|| "genesis_hash contains invalid hexadecimal data")?;
    }
    Ok(hash)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
