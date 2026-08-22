use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use jamscript_codegen_rust::{generate_no_std_rust_with_context, MiniJamContext};
use jamscript_ir::abi_for;
use jamscript_parser::parse_service_with_native_modules;
use jamscript_target_minijam::{MiniJamTarget, NativeModule};
use serde::Deserialize;
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    package: Package,
    target: Option<Target>,
    native: Option<BTreeMap<String, NativeConfig>>,
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
    service_id: Option<u32>,
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
            println!("checked {} action `{}`", path.display(), ir.actions[0].name);
            Ok(())
        }
        CommandKind::Abi { path } => {
            let (_, ir) = load(&path)?;
            println!("{}", serde_json::to_string_pretty(&abi_for(&ir))?);
            Ok(())
        }
        CommandKind::Build { path, output } => build(&path, &output),
    }
}

fn load(path: &Path) -> Result<(Manifest, jamscript_ir::ServiceIr)> {
    let manifest_path = path.join("jamscript.toml");
    let manifest: Manifest = toml::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )?;
    if manifest.package.language != "0.1" {
        bail!(
            "unsupported language version `{}`",
            manifest.package.language
        );
    }
    let source_path = path.join(&manifest.package.entry);
    let source = fs::read_to_string(&source_path)
        .with_context(|| format!("reading {}", source_path.display()))?;
    let native_modules = manifest
        .native
        .as_ref()
        .map(|modules| modules.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let ir = parse_service_with_native_modules(
        &source,
        &manifest.package.name,
        &manifest.package.version,
        &native_modules,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", source_path.display()))?;
    Ok((manifest, ir))
}

fn new_project(name: &str) -> Result<()> {
    let root = PathBuf::from(name);
    if root.exists() {
        bail!("directory already exists: {}", root.display());
    }
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("jamscript.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/service.ts\"\nlanguage = \"0.1\"\n"))?;
    fs::write(root.join("src/service.ts"), "import { action, wallet, u64 } from \"jam\";\n\nexport const increment = action({\n  auth: wallet(),\n  input: { value: u64 },\n  execute(ctx, input) {\n    return input.value + 1;\n  },\n});\n")?;
    println!("created {}", root.display());
    Ok(())
}

fn build(path: &Path, output: &Path) -> Result<()> {
    let (manifest, ir) = load(path)?;
    let minijam = manifest
        .target
        .as_ref()
        .and_then(|target| target.minijam.as_ref());
    let context = MiniJamContext {
        service_id: minijam.and_then(|target| target.service_id).unwrap_or(0),
        genesis_hash: minijam
            .and_then(|target| target.genesis_hash.as_deref())
            .map(parse_hash)
            .transpose()?
            .unwrap_or([0; 32]),
    };
    fs::create_dir_all(output)?;
    let abi = abi_for(&ir);
    fs::write(
        output.join("service.abi.json"),
        serde_json::to_vec_pretty(&abi)?,
    )?;
    fs::write(
        output.join("generated_service.rs"),
        generate_no_std_rust_with_context(&ir, context).map_err(|e| anyhow::anyhow!(e))?,
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
    let metadata = MiniJamTarget::from_sdk_root(sdk_root)
        .build_probe(&project_root, &ir, context, output, &native_modules)
        .context("MiniJAM target build")?;
    fs::write(
        output.join("build.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!("built {}", output.display());
    Ok(())
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
