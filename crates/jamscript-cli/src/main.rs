use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use jamscript_codegen_rust::{
    generate_builder_application_rust, ManagementPolicyConfig, PortableServiceContext,
};
use jamscript_deployment::{
    load_service_artifact, redact_url, register_backend_service, resolve_network,
    validate_networks, CurlJsonRpcTransport, DeploymentConfig, DeploymentEngine, JsonRpcTransport,
    NetworkConfig, NetworkOverrides,
};
use jamscript_ir::abi_for_language;
use jamscript_parser::parse_service_v02;
use jamscript_target_jam::{verify_deployment_bundle, JamTarget, NativeModule};
use jamscript_toolchain::{lld_executable_name, ToolchainManager};
use polkavm::{
    BackendKind, Config as PvmConfig, Engine, Linker, MemoryAccessError, Module, ModuleConfig, Reg,
};
use serde::Deserialize;
use service_runtime_core::ServiceKeyV1;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "jams",
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
        #[arg(long)]
        offline: bool,
    },
    Toolchain {
        #[command(subcommand)]
        command: ToolchainCommand,
    },
    Doctor {
        #[arg(long)]
        json: bool,
    },
    Run {
        #[arg(default_value = "dist/service.pvm")]
        artifact: PathBuf,
        #[arg(long, default_value = "minijam_refine")]
        export: String,
        #[arg(long)]
        result: Option<PathBuf>,
    },
    Inspect {
        #[arg(default_value = "dist")]
        bundle: PathBuf,
    },
    Deploy {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        network: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        deployment_rpc: Option<String>,
        #[arg(long)]
        node_rpc: Option<String>,
        #[arg(long)]
        backend_rpc: Option<String>,
        #[arg(long, default_value = "dist")]
        artifact: PathBuf,
        #[arg(long, default_value = "120s")]
        timeout: String,
        #[arg(long)]
        json: bool,
    },
    Network {
        #[command(subcommand)]
        command: NetworkCommand,
    },
    Backend {
        #[command(subcommand)]
        command: BackendCommand,
    },
}

#[derive(Subcommand)]
enum ToolchainCommand {
    Status {
        #[arg(long)]
        json: bool,
    },
    Install,
    Verify,
    Path,
}

#[derive(Subcommand)]
enum NetworkCommand {
    List {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Show {
        name: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum BackendCommand {
    Start {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "local")]
        network: String,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        node_rpc: Option<String>,
        #[arg(long)]
        formal_rpc: Option<String>,
        #[arg(long = "cors-origin", action = clap::ArgAction::Append)]
        cors_origins: Vec<String>,
    },
}

struct DeployOptions {
    path: PathBuf,
    network: Option<String>,
    kind: Option<String>,
    deployment_rpc: Option<String>,
    node_rpc: Option<String>,
    backend_rpc: Option<String>,
    artifact: PathBuf,
    timeout: String,
    json: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    package: Package,
    compiler: Option<CompilerConfig>,
    target: Option<Target>,
    native: Option<BTreeMap<String, NativeConfig>>,
    management: Option<ManagementConfig>,
    networks: Option<BTreeMap<String, NetworkConfig>>,
    deployment: Option<DeploymentConfig>,
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
    jam: Option<JamConfig>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JamConfig {
    /// Optional deployment domain used by the runtime signing context.
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
        CommandKind::Build {
            path,
            output,
            offline,
        } => {
            if offline {
                std::env::set_var("JAMSCRIPT_OFFLINE", "1");
            }
            build(&path, &output)
        }
        CommandKind::Toolchain { command } => toolchain_command(command),
        CommandKind::Doctor { json } => doctor(json),
        CommandKind::Run {
            artifact,
            export,
            result,
        } => run_artifact(&artifact, &export, result.as_deref()),
        CommandKind::Inspect { bundle } => inspect(&bundle),
        CommandKind::Deploy {
            path,
            network,
            kind,
            deployment_rpc,
            node_rpc,
            backend_rpc,
            artifact,
            timeout,
            json,
        } => deploy(DeployOptions {
            path,
            network,
            kind,
            deployment_rpc,
            node_rpc,
            backend_rpc,
            artifact,
            timeout,
            json,
        }),
        CommandKind::Network { command } => network_command(command),
        CommandKind::Backend { command } => match command {
            BackendCommand::Start {
                path,
                network,
                bind,
                data_dir,
                node_rpc,
                formal_rpc,
                cors_origins,
            } => backend_start(BackendStartOptions {
                path,
                network,
                bind,
                data_dir,
                node_rpc,
                formal_rpc,
                cors_origins,
            }),
        },
    }
}

struct BackendStartOptions {
    path: PathBuf,
    network: String,
    bind: Option<String>,
    data_dir: Option<PathBuf>,
    node_rpc: Option<String>,
    formal_rpc: Option<String>,
    cors_origins: Vec<String>,
}

fn backend_start(options: BackendStartOptions) -> Result<()> {
    let project_root = options
        .path
        .canonicalize()
        .with_context(|| format!("locating JamScript project {}", options.path.display()))?;
    let manifest = read_manifest(&project_root)?;
    let config = manifest
        .networks
        .as_ref()
        .and_then(|networks| networks.get(&options.network))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "NETWORK_NOT_FOUND: network '{}' is not configured",
                options.network
            )
        })?;
    if config.kind != "minijam" {
        bail!(
            "backend network '{}' must use kind = \"minijam\"",
            options.network
        );
    }
    let node_rpc = options
        .node_rpc
        .or_else(|| config.node_rpc.clone())
        .or_else(|| std::env::var("JAMSCRIPT_NODE_RPC").ok())
        .ok_or_else(|| anyhow::anyhow!("backend start requires node_rpc"))?;
    let formal_rpc = options
        .formal_rpc
        .or_else(|| config.deployment_rpc.clone())
        .or_else(|| std::env::var("JAMSCRIPT_FORMAL_RPC").ok())
        .ok_or_else(|| anyhow::anyhow!("backend start requires deployment_rpc/formal_rpc"))?;
    if let Some(expected) = config.genesis_hash.as_deref() {
        let actual = CurlJsonRpcTransport
            .call(
                &node_rpc,
                "chain_getBlockHash",
                serde_json::json!([0]),
                std::time::Duration::from_secs(10),
                false,
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let actual = actual
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("node genesis RPC returned no hash"))?;
        if parse_hash(expected)? != parse_hash(actual)? {
            bail!("backend network genesis hash does not match jamscript.toml");
        }
    }
    let binary = find_backend_binary()?;
    let mut command = std::process::Command::new(&binary);
    command
        .arg("--network")
        .arg("local")
        .arg("--node-rpc")
        .arg(node_rpc)
        .arg("--formal-rpc")
        .arg(formal_rpc);
    if let Some(bind) = options
        .bind
        .or_else(|| std::env::var("JAMSCRIPT_BACKEND_BIND").ok())
    {
        command.arg("--bind").arg(bind);
    }
    if let Some(data_dir) = options.data_dir.or_else(|| {
        std::env::var("JAMSCRIPT_BACKEND_DATA")
            .ok()
            .map(PathBuf::from)
    }) {
        command.arg("--data-dir").arg(data_dir);
    }
    for origin in options.cors_origins {
        command.arg("--cors-origin").arg(origin);
    }
    println!(
        "Starting JamScript backend in the foreground using network '{}'",
        options.network
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(anyhow::anyhow!(
            "starting backend {}: {error}",
            binary.display()
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .with_context(|| format!("starting backend {}", binary.display()))?;
        if !status.success() {
            bail!("backend exited with {status}");
        }
        Ok(())
    }
}

fn find_backend_binary() -> Result<PathBuf> {
    if let Ok(current) = std::env::current_exe() {
        if let Some(candidate) = current
            .parent()
            .map(|path| path.join("jamscript-service-backend"))
        {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join("jamscript-service-backend");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Some(path) = std::env::var_os("JAMSCRIPT_BACKEND_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "JAMSCRIPT_BACKEND_BIN does not point to a file: {}",
            path.display()
        );
    }
    bail!("JamScript backend binary is not installed. Install the backend package or use Docker.")
}

fn toolchain_command(command: ToolchainCommand) -> Result<()> {
    let manager = ToolchainManager::new()?;
    match command {
        ToolchainCommand::Status { json } => {
            let status = manager.status();
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("Toolchain: {}", status.toolchain_id);
                println!("Platform: {}", status.platform);
                println!("Installed: {}", yes_no(status.installed));
                println!("Verified: {}", yes_no(status.verified));
                if let Some(error) = status.error {
                    println!("Status: {error}");
                }
            }
            Ok(())
        }
        ToolchainCommand::Install => {
            let toolchain = manager.install()?;
            println!("Toolchain verified at {}", toolchain.root.display());
            Ok(())
        }
        ToolchainCommand::Verify => {
            let toolchain = manager.verify()?;
            println!("Toolchain verified at {}", toolchain.root.display());
            Ok(())
        }
        ToolchainCommand::Path => {
            println!("{}", manager.path()?.display());
            Ok(())
        }
    }
}

fn doctor(json: bool) -> Result<()> {
    let manager = ToolchainManager::new()?;
    let status = manager.status();
    let root = status.path.clone();
    let lld_name = lld_executable_name(&status.platform);
    let jams_binary = std::env::current_exe().ok();
    let tool_paths = root.as_ref().map(|root| {
        serde_json::json!({
            "rustc": root.join("bin/rustc"),
            "cargo": root.join("bin/cargo"),
            "node": root.join("bin/node"),
            "scriptc": root.join("scriptc"),
            "clang": root.join("bin/clang"),
            "llvm_ar": root.join("bin/llvm-ar"),
            "lld": root.join("bin").join(lld_name),
            "readelf": root.join("bin/llvm-readelf"),
            "host_linker": root.join("bin/jamscript-host-linker"),
            "polkavm": root.join("toolchains/polkavm.lock"),
            "jam_sdk": root.join("targets/jam/sdk"),
            "toolchain_root": root,
            "jams": jams_binary,
        })
    });
    let managed_paths_only = root.as_ref().is_some_and(|root| {
        [
            root.join("bin/rustc"),
            root.join("bin/cargo"),
            root.join("bin/node"),
            root.join("scriptc"),
            root.join("bin/clang"),
            root.join("bin/llvm-ar"),
            root.join("bin").join(lld_name),
            root.join("bin/llvm-readelf"),
            root.join("bin/jamscript-host-linker"),
            root.join("toolchains/polkavm.lock"),
            root.join("targets/jam/sdk"),
        ]
        .iter()
        .all(|path| path.starts_with(root) && (path.is_file() || path.is_dir()))
    });
    let canonical_ready = status.installed && status.verified && managed_paths_only;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "jamscript": { "version": env!("CARGO_PKG_VERSION") },
                "jams_binary": jams_binary,
                "toolchain": {
                    "id": status.toolchain_id,
                    "platform": status.platform,
                    "root": root,
                    "manifest": root.as_ref().map(|path| path.join("manifest.json")),
                    "digest": status.sha256,
                    "verified": status.verified,
                },
                "resolved_tools": tool_paths,
                "host_dependency_leakage": if managed_paths_only { "PASS" } else { "FAIL" },
                "canonical_build_readiness": if canonical_ready { "PASS" } else { "FAIL" },
                "canonical": canonical_ready,
                "toolchain_home": root,
                "node": root.as_ref().map(|path| path.join("bin/node")),
                "rustc": root.as_ref().map(|path| path.join("bin/rustc")),
                "cargo": root.as_ref().map(|path| path.join("bin/cargo")),
                "clang": root.as_ref().map(|path| path.join("bin/clang")),
                "scriptc": root.as_ref().map(|path| path.join("scriptc")),
                "offline": matches!(
                    std::env::var("JAMSCRIPT_OFFLINE").as_deref(),
                    Ok("1") | Ok("true") | Ok("yes")
                ),
                "error": status.error,
            }))?
        );
        if !canonical_ready {
            bail!(
                "canonical build readiness failed; run `jams toolchain install` and `jams doctor`"
            );
        }
        return Ok(());
    }
    println!("JamScript CLI: {}", env!("CARGO_PKG_VERSION"));
    println!("Language: 0.2\n");
    println!("Host:\n{}\n", status.platform);
    println!(
        "Toolchain:\n{}\ninstalled: {}\nverified: {}\n",
        status.toolchain_id,
        yes_no(status.installed),
        yes_no(status.verified)
    );
    if let Some(root) = &root {
        println!("Bundle root:\n{}", root.display());
        println!("Manifest:\n{}", root.join("manifest.json").display());
        println!("Digest:\n{}", status.sha256);
        for (name, path) in [
            ("Rust compiler", root.join("bin/rustc")),
            ("Cargo", root.join("bin/cargo")),
            ("Node", root.join("bin/node")),
            ("ScriptC", root.join("scriptc")),
            ("Clang", root.join("bin/clang")),
            ("LLVM/Clang linker", root.join("bin").join(lld_name)),
            ("LLVM ELF inspector", root.join("bin/llvm-readelf")),
            (
                "Managed host linker",
                root.join("bin/jamscript-host-linker"),
            ),
            ("PolkaVM lock", root.join("toolchains/polkavm.lock")),
            ("JAM SDK", root.join("targets/jam/sdk")),
        ] {
            println!(
                "{name}:\n{} {}",
                path.display(),
                check_marker(path.is_file() || path.is_dir())
            );
        }
    }
    println!(
        "\nHost dependency leakage: {}",
        check_marker(managed_paths_only)
    );
    println!(
        "Canonical build readiness: {}",
        check_marker(canonical_ready)
    );
    println!(
        "Node:\n{} {}",
        manager.manifest().node_version,
        check_marker(status.verified)
    );
    println!(
        "\nLLVM:\n{} {}",
        manager.manifest().clang_version,
        check_marker(status.verified)
    );
    println!(
        "\nRust:\n{} {}",
        manager.manifest().rust_toolchain,
        check_marker(status.verified)
    );
    println!(
        "\nPolkaVM:\n{} {}",
        manager.manifest().polkavm_linker,
        check_marker(status.verified)
    );
    println!(
        "\nJAM target:\n{} {}",
        manager.manifest().jam_target_version,
        check_marker(status.verified)
    );
    if !canonical_ready {
        bail!("canonical build readiness failed; run `jams toolchain install` and `jams doctor`");
    }
    Ok(())
}

fn run_artifact(artifact: &Path, export: &str, result_path: Option<&Path>) -> Result<()> {
    let bytes = fs::read(artifact)
        .with_context(|| format!("reading PVM artifact {}", artifact.display()))?;
    let mut config = PvmConfig::new();
    config.set_backend(Some(BackendKind::Interpreter));
    let engine = Engine::new(&config).context("creating PolkaVM interpreter")?;
    let module =
        Module::new(&engine, &ModuleConfig::new(), bytes.into()).context("loading PVM artifact")?;
    let payload = empty_refine_input();
    let mut linker: Linker<(), MemoryAccessError> = Linker::new();
    linker
        .define_untyped("minijam_fetch", move |caller| {
            let output = caller.instance.reg(Reg::A0) as u32;
            let offset = caller.instance.reg(Reg::A1) as usize;
            let capacity = caller.instance.reg(Reg::A2) as usize;
            let mode = caller.instance.reg(Reg::A3);
            let index = caller.instance.reg(Reg::A4);
            let value = if mode == 13 && index == 0 {
                payload.as_slice()
            } else {
                &[]
            };
            if value.is_empty() && !(mode == 13 && index == 0) {
                caller.instance.set_reg(Reg::A0, u64::MAX);
            } else if offset > value.len() {
                caller.instance.set_reg(Reg::A0, u64::MAX);
            } else {
                let remaining = &value[offset..];
                if remaining.len() <= capacity {
                    caller.instance.write_memory(output, remaining)?;
                }
                caller.instance.set_reg(Reg::A0, remaining.len() as u64);
            }
            Ok(())
        })
        .context("registering deterministic JAM host shim")?;
    linker.define_fallback(|caller, _| {
        caller.instance.set_reg(Reg::A0, u64::MAX);
        Ok(())
    });
    let pre = linker
        .instantiate_pre(&module)
        .context("linking PVM artifact")?;
    let mut instance = pre.instantiate().context("instantiating PVM artifact")?;
    instance.set_gas(5_000_000);
    if export == "minijam_accumulate" {
        instance
            .call_typed_and_get_result::<(), _>(&mut (), export, ())
            .map_err(|error| anyhow::anyhow!("PVM execution: {error:?}"))?;
    } else if matches!(
        export,
        "minijam_refine" | "jamscript_plan_v1" | "jamscript_backend_metadata_v1"
    ) {
        instance
            .call_typed_and_get_result::<u64, _>(&mut (), export, ())
            .map_err(|error| anyhow::anyhow!("PVM execution: {error:?}"))?;
        let pointer = instance.reg(Reg::A0) as u32;
        let size = instance.reg(Reg::A1) as u32;
        let output = instance
            .read_memory(pointer, size)
            .context("reading PVM execution result")?;
        if output.is_empty() {
            bail!("PVM execution returned an empty result");
        }
        if let Some(path) = result_path {
            fs::write(path, &output).with_context(|| format!("writing {}", path.display()))?;
        }
        println!("PVM_RESULT_BYTES={}", output.len());
    } else {
        let value = instance
            .call_typed_and_get_result::<u64, _>(&mut (), export, ())
            .map_err(|error| anyhow::anyhow!("PVM execution: {error:?}"))?;
        println!("PVM_RESULT={value}");
    }
    println!("PVM_EXPORT={export}");
    println!("PVM_EXECUTION=PASS");
    Ok(())
}

fn empty_refine_input() -> Vec<u8> {
    let plan = [1u8, 0, 0, 0, 0];
    let mut witness = Vec::with_capacity(46);
    witness.push(1);
    witness.extend_from_slice(&[0; 32]);
    witness.extend_from_slice(&(plan.len() as u32).to_le_bytes());
    witness.extend_from_slice(&plan);
    witness.extend_from_slice(&0u32.to_le_bytes());
    let mut input = Vec::with_capacity(55);
    input.push(1);
    input.extend_from_slice(&(witness.len() as u32).to_le_bytes());
    input.extend_from_slice(&witness);
    input.extend_from_slice(&0u32.to_le_bytes());
    input
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
fn check_marker(value: bool) -> &'static str {
    if value {
        "PASS"
    } else {
        "FAIL"
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

fn network_command(command: NetworkCommand) -> Result<()> {
    match command {
        NetworkCommand::List { path, json } => {
            let manifest = read_manifest(&path)?;
            let networks = manifest.networks.unwrap_or_default();
            if json {
                let values = networks
                    .iter()
                    .map(|(name, config)| {
                        serde_json::json!({
                            "name": name,
                            "kind": config.kind,
                            "deploymentRpc": config.deployment_rpc.as_deref().map(redact_url),
                            "nodeRpc": config.node_rpc.as_deref().map(redact_url),
                            "default": manifest.deployment.as_ref()
                                .and_then(|deployment| deployment.default_network.as_deref())
                                == Some(name.as_str()),
                        })
                    })
                    .collect::<Vec<_>>();
                println!("{}", serde_json::to_string_pretty(&values)?);
            } else if networks.is_empty() {
                println!("No deployment networks configured.");
                println!("Add [networks.<name>] to jamscript.toml.");
            } else {
                println!("{:<20} {:<10} DEPLOYMENT", "NAME", "KIND");
                for (name, config) in networks {
                    println!(
                        "{:<20} {:<10} {}",
                        name,
                        config.kind,
                        config
                            .deployment_rpc
                            .as_deref()
                            .map(redact_url)
                            .unwrap_or_else(|| "not configured".into())
                    );
                }
            }
            Ok(())
        }
        NetworkCommand::Show { name, path, json } => {
            let manifest = read_manifest(&path)?;
            let config = manifest
                .networks
                .as_ref()
                .and_then(|networks| networks.get(&name))
                .ok_or_else(|| {
                    anyhow::anyhow!("NETWORK_NOT_FOUND: network '{name}' is not configured")
                })?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "name": name,
                        "kind": config.kind,
                        "deploymentRpc": config.deployment_rpc.as_deref().map(redact_url),
                        "nodeRpc": config.node_rpc.as_deref().map(redact_url),
                        "backendRpc": config.backend_rpc.as_deref().map(redact_url),
                        "genesisHash": config.genesis_hash,
                        "default": manifest.deployment.as_ref()
                            .and_then(|deployment| deployment.default_network.as_deref())
                            == Some(name.as_str()),
                    }))?
                );
            } else {
                println!("Name\n  {name}");
                println!("Kind\n  {}", config.kind);
                println!(
                    "Deployment RPC\n  {}",
                    config
                        .deployment_rpc
                        .as_deref()
                        .map(redact_url)
                        .unwrap_or_else(|| "not configured".into())
                );
                println!(
                    "Node RPC\n  {}",
                    config
                        .node_rpc
                        .as_deref()
                        .map(redact_url)
                        .unwrap_or_else(|| "not configured".into())
                );
                println!(
                    "Backend RPC\n  {}",
                    config
                        .backend_rpc
                        .as_deref()
                        .map(redact_url)
                        .unwrap_or_else(|| "not configured".into())
                );
                println!(
                    "Genesis pin\n  {}",
                    config.genesis_hash.as_deref().unwrap_or("not configured")
                );
            }
            Ok(())
        }
    }
}

fn deploy(options: DeployOptions) -> Result<()> {
    let DeployOptions {
        path,
        network,
        kind,
        deployment_rpc,
        node_rpc,
        backend_rpc,
        artifact,
        timeout,
        json,
    } = options;
    let project_root = path
        .canonicalize()
        .with_context(|| format!("locating JamScript project {}", path.display()))?;
    let manifest = read_manifest(&project_root)?;
    let artifact_dir = if artifact.is_absolute() {
        artifact
    } else {
        project_root.join(artifact)
    };
    let service_artifact =
        load_service_artifact(&artifact_dir).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let resolved = resolve_network(
        manifest.networks.as_ref(),
        manifest.deployment.as_ref(),
        NetworkOverrides {
            network,
            kind,
            deployment_rpc,
            node_rpc,
            backend_rpc,
        },
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let timeout = parse_duration(&timeout)?;
    if !json {
        println!("JamScript Deployment\n");
        println!("Network");
        println!("  Name: {}", resolved.display_name());
        println!("  Kind: {}", resolved.kind);
        println!("  Endpoint: {}", redact_url(&resolved.deployment_rpc));
        println!(
            "  Identity: {}",
            if resolved.genesis_hash.is_some() {
                "pinned; verifying"
            } else {
                "unpinned"
            }
        );
        println!("\nArtifact");
        println!("  {}", service_artifact.blob_path.display());
        println!(
            "  Code hash: {}",
            jamscript_deployment::hash_hex(&service_artifact.code_hash)
        );
        println!("  Artifact verification: PASS");
        println!("\nSubmitting deployment...");
    }
    let backend_rpc = resolved.backend_rpc.clone();
    let result = DeploymentEngine::new(CurlJsonRpcTransport)
        .deploy(resolved, service_artifact.clone(), timeout)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let record = jamscript_deployment::write_deployment_record(&project_root, &result)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let backend_registration = if let Some(endpoint) = backend_rpc {
        Some(
            match register_backend_service(
                &CurlJsonRpcTransport,
                &endpoint,
                result.service_id,
                &service_artifact,
                timeout,
            ) {
                Ok(value) => serde_json::json!({"status": "PASS", "result": value}),
                Err(error) => serde_json::json!({"status": "FAILED", "error": error.to_string()}),
            },
        )
    } else {
        None
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "PASS",
                "network": {
                    "name": result.network.name,
                    "kind": result.network.kind,
                    "genesisHash": result.network.genesis_hash,
                    "identity": format!("{:?}", result.network.verification).to_ascii_lowercase(),
                },
                "serviceId": result.service_id,
                "codeHash": result.code_hash,
                "finalized": result.finalized,
                "finalizedBlock": result.finalized_context,
                "operationId": result.operation_id,
                "backendRegistration": backend_registration,
                "record": record,
            }))?
        );
    } else {
        println!("  PASS");
        println!("\nService");
        println!("  ID: {}", result.service_id);
        println!("\nFinalization");
        println!("  {}", if result.finalized { "PASS" } else { "FAIL" });
        println!(
            "  Block: {}",
            result
                .finalized_context
                .as_ref()
                .map(|context| format!("{} ({})", context.block_hash, context.block_number))
                .unwrap_or_else(|| "not reported".into())
        );
        println!("Code identity\n  PASS");
        println!(
            "Network identity\n  {}",
            if result.network.verification == jamscript_deployment::IdentityVerification::Verified {
                "VERIFIED"
            } else {
                "UNPINNED"
            }
        );
        println!("\nDeployment record\n  {}", record.display());
        println!(
            "\nBackend registration\n  {}",
            if backend_registration.is_some() {
                if backend_registration
                    .as_ref()
                    .and_then(|value| value.get("status"))
                    .and_then(serde_json::Value::as_str)
                    == Some("PASS")
                {
                    "PASS"
                } else {
                    "FAILED (on-chain deployment remains finalized)"
                }
            } else {
                "not configured"
            }
        );
        println!("\nDEPLOYMENT=PASS");
    }
    Ok(())
}

fn parse_duration(value: &str) -> Result<std::time::Duration> {
    let value = value.trim();
    let split = value
        .trim_end_matches(|character: char| character.is_ascii_alphabetic())
        .len();
    let (number, unit) = value.split_at(split);
    let seconds = number
        .parse::<u64>()
        .with_context(|| format!("invalid deployment timeout '{value}'"))?;
    let multiplier = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        _ => bail!("invalid deployment timeout unit '{unit}'; use seconds, minutes, or hours"),
    };
    let seconds = seconds
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("deployment timeout is too large"))?;
    if seconds == 0 {
        bail!("deployment timeout must be greater than zero");
    }
    Ok(std::time::Duration::from_secs(seconds))
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    let manifest_path = path.join("jamscript.toml");
    let manifest: Manifest = toml::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )?;
    validate_networks(manifest.networks.as_ref(), manifest.deployment.as_ref())
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
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
    Ok(manifest)
}

fn load(path: &Path) -> Result<(Manifest, jamscript_ir::ServiceIr)> {
    let manifest = read_manifest(path)?;
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
    let managed_toolchain = if std::env::var("JAMSCRIPT_DEV_TOOLCHAIN").as_deref() == Ok("1") {
        None
    } else {
        let manager = ToolchainManager::new()?;
        let status = manager.status();
        if !status.installed {
            println!(
                "JamScript toolchain is not installed for {}.",
                status.platform
            );
            println!("Installing the exact managed toolchain...");
        }
        Some(manager.resolve()?)
    };
    let jam = manifest
        .target
        .as_ref()
        .and_then(|target| target.jam.as_ref());
    let (service_key, service_instance_id) = load_service_identity(path)?;
    let management_policy = resolve_management_policy(manifest.management.as_ref(), &service_key)?;
    let context = PortableServiceContext {
        service_key: service_key.into_bytes(),
        service_instance_id,
        management_policy,
        genesis_hash: jam
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
    let project_root = path
        .canonicalize()
        .with_context(|| format!("canonicalizing project root {}", path.display()))?;
    let native_modules = resolve_native_modules(&project_root, manifest.native.as_ref())?;
    let target = match managed_toolchain.as_ref() {
        Some(toolchain) => JamTarget::from_installed_toolchain(toolchain),
        None => JamTarget::new(),
    };
    target
        .build_scriptc_probe(&project_root, &ir, context, output, &native_modules)
        .context("JAM target build")?;
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
