use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use jamscript_codegen_rust::generate_no_std_rust;
use jamscript_ir::abi_for;
use jamscript_parser::parse_service;
use jamscript_target_minijam::MiniJamTarget;
use serde::Deserialize;
use std::{
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
struct Manifest {
    package: Package,
    target: Option<Target>,
}
#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    version: String,
    entry: String,
    language: String,
}
#[derive(Debug, Deserialize)]
struct Target {
    minijam: Option<MiniJamConfig>,
}
#[derive(Debug, Deserialize)]
struct MiniJamConfig {
    sdk_root: Option<String>,
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
    let ir = parse_service(&source, &manifest.package.name, &manifest.package.version)
        .map_err(|e| anyhow::anyhow!("{}: {e}", source_path.display()))?;
    Ok((manifest, ir))
}

fn new_project(name: &str) -> Result<()> {
    let root = PathBuf::from(name);
    if root.exists() {
        bail!("directory already exists: {}", root.display());
    }
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("jamscript.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"src/service.ts\"\nlanguage = \"0.1\"\n\n[target.minijam]\nnetwork = \"stage0\"\n"))?;
    fs::write(root.join("src/service.ts"), "import { action, wallet, u64 } from \"jam\";\n\nexport const increment = action({\n  auth: wallet(),\n  input: { value: u64 },\n  compute(ctx, input) {\n    return input.value + 1;\n  },\n});\n")?;
    println!("created {}", root.display());
    Ok(())
}

fn build(path: &Path, output: &Path) -> Result<()> {
    let (manifest, ir) = load(path)?;
    fs::create_dir_all(output)?;
    let abi = abi_for(&ir);
    fs::write(
        output.join("service.abi.json"),
        serde_json::to_vec_pretty(&abi)?,
    )?;
    fs::write(
        output.join("generated_service.rs"),
        generate_no_std_rust(&ir).map_err(|e| anyhow::anyhow!(e))?,
    )?;
    let sdk_root = manifest
        .target
        .and_then(|target| target.minijam)
        .and_then(|target| target.sdk_root)
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
    let metadata = MiniJamTarget::from_sdk_root(sdk_root)
        .build_probe(&ir, output)
        .context("MiniJAM target build")?;
    fs::write(
        output.join("build.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!("built {}", output.display());
    Ok(())
}
