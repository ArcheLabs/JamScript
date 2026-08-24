use anyhow::{bail, Context, Result};
use polkavm::{Engine, Linker, Module, ModuleConfig};
use polkavm_linker::{program_from_elf, Config as LinkConfig, TargetInstructionSet};
use serde::Serialize;
use service_build_polkavm::{PolkaVmBuildConfig, PolkaVmBuildRequest, PolkaVmBuilder};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const GAS_LIMIT: i64 = 5_000_000;

#[derive(Serialize)]
struct StageReport {
    stage: u64,
    result: &'static str,
    gas: Option<i64>,
    allocation_count: Option<u64>,
    requested_bytes: Option<u64>,
    high_water_mark: Option<u64>,
    error: Option<String>,
}

fn main() -> Result<()> {
    let repo = repo_root();
    let scriptc = repo.join("toolchains/scriptc");
    let node = env::var_os("SCRIPTC_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| "node".into());
    let node_version = run_capture(&node, &["--version"], &scriptc)?;
    if !node_version.trim_start().starts_with("v24.") {
        bail!("M0.6 requires Node 24.x, got {}", node_version.trim());
    }
    let generated = scriptc.join("m0/out-m06");
    fs::create_dir_all(&generated)?;
    let status = Command::new(&node)
        .current_dir(&scriptc)
        .env("SCRIPTC_M0_OUT", &generated)
        .arg("m0/run.mjs")
        .status()
        .context("running ScriptC M0 with the pinned Node executable")?;
    if !status.success() {
        bail!("ScriptC M0 generation failed");
    }
    let scalar = generated.join("scalar/scalar.lib.c");
    let runtime = scriptc.join("node_modules/@scriptc/runtime");
    if !scalar.is_file() || !runtime.is_dir() {
        bail!("generated scalar C or ScriptC runtime is missing");
    }
    env::set_var("SCRIPTC_M06_SCALAR_C", &scalar);
    env::set_var("SCRIPTC_M06_RUNTIME", &runtime);

    let output = repo.join("target/pvm-scriptc-m0");
    fs::create_dir_all(&output)?;
    env::set_var(
        "SCRIPTC_M06_DEPENDENCY_REPORT",
        output.join("runtime-dependencies.json"),
    );
    let manifest = repo.join("tools/pvm-scriptc-m0-guest/Cargo.toml");
    let artifacts = PolkaVmBuilder::new(PolkaVmBuildConfig {
        diagnostic: true,
        rustflags: Some("-C link-arg=-nostdlib -C link-arg=--gc-sections".into()),
        ..Default::default()
    })
    .build(&PolkaVmBuildRequest {
        manifest_path: manifest,
        output_dir: output.clone(),
        native_archives: Vec::new(),
        required_exports: vec!["probe_entry".into()],
        require_relocations: false,
    })?;
    let blob = program_from_elf(
        LinkConfig::default(),
        TargetInstructionSet::JamV1,
        &fs::read(&artifacts.elf)?,
    )
    .map_err(|error| anyhow::anyhow!("converting ScriptC ELF to PVM: {error:?}"))?;
    fs::write(output.join("scriptc-m0.pvm"), &blob)?;
    fs::copy(&artifacts.elf, output.join("scalar.elf"))?;
    fs::write(output.join("scalar.polkavm"), &blob)?;
    fs::write(output.join("scalar.pvm"), &blob)?;

    let engine = make_engine()?;
    let mut reports = Vec::new();
    for stage in 1..=4 {
        match run_stage(&engine, &blob, stage) {
            Ok(metrics) => reports.push(StageReport {
                stage,
                result: "PASS",
                gas: Some(metrics.gas),
                allocation_count: Some(metrics.allocation_count),
                requested_bytes: Some(metrics.requested_bytes),
                high_water_mark: Some(metrics.high_water_mark),
                error: None,
            }),
            Err(error) => reports.push(StageReport {
                stage,
                result: classify(&error),
                gas: None,
                allocation_count: None,
                requested_bytes: None,
                high_water_mark: None,
                error: Some(error.to_string()),
            }),
        }
    }
    fs::write(
        output.join("m06.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!("Node: {}", node_version.trim());
    println!("ELF: {}", artifacts.elf.display());
    for report in &reports {
        println!(
            "stage {}: {} gas={} alloc={} requested={} high_water={}{}",
            report.stage,
            report.result,
            report.gas.map_or_else(|| "-".into(), |gas| gas.to_string()),
            report
                .allocation_count
                .map_or_else(|| "-".into(), |value| value.to_string()),
            report
                .requested_bytes
                .map_or_else(|| "-".into(), |value| value.to_string()),
            report
                .high_water_mark
                .map_or_else(|| "-".into(), |value| value.to_string()),
            report
                .error
                .as_deref()
                .map_or_else(String::new, |error| format!(" error={error}"))
        );
    }
    if reports.iter().any(|report| report.result != "PASS") {
        bail!("ScriptC M0.6 PVM probe failed");
    }
    Ok(())
}

struct StageMetrics {
    gas: i64,
    allocation_count: u64,
    requested_bytes: u64,
    high_water_mark: u64,
}

fn run_stage(engine: &Engine, blob: &[u8], stage: u64) -> Result<StageMetrics> {
    let config = module_config();
    let module = Module::new(engine, &config, blob.to_vec().into())?;
    let linker: Linker<(), core::convert::Infallible> = Linker::new();
    let pre = linker.instantiate_pre(&module)?;
    let mut instance = pre.instantiate()?;
    instance.set_gas(GAS_LIMIT);
    let value = instance
        .call_typed_and_get_result::<u64, _>(&mut (), "probe_entry", (stage,))
        .map_err(|error| anyhow::anyhow!("PVM execution: {error:?}"))?;
    if value != 1 {
        bail!("probe returned {value}");
    }
    let gas = GAS_LIMIT - instance.gas();
    let allocation_count = instance
        .call_typed_and_get_result::<u64, _>(&mut (), "probe_allocation_count", ())
        .map_err(|error| anyhow::anyhow!("reading probe_allocation_count: {error:?}"))?;
    let requested_bytes = instance
        .call_typed_and_get_result::<u64, _>(&mut (), "probe_requested_bytes", ())
        .map_err(|error| anyhow::anyhow!("reading probe_requested_bytes: {error:?}"))?;
    let high_water_mark = instance
        .call_typed_and_get_result::<u64, _>(&mut (), "probe_high_water_mark", ())
        .map_err(|error| anyhow::anyhow!("reading probe_high_water_mark: {error:?}"))?;
    Ok(StageMetrics {
        gas,
        allocation_count,
        requested_bytes,
        high_water_mark,
    })
}

fn make_engine() -> Result<Engine> {
    let mut config = polkavm::Config::new();
    config.set_backend(Some(polkavm::BackendKind::Interpreter));
    Ok(Engine::new(&config)?)
}

fn module_config() -> ModuleConfig {
    let mut config = ModuleConfig::new();
    config.set_gas_metering(Some(polkavm::GasMeteringKind::Sync));
    config
}

fn classify(error: &anyhow::Error) -> &'static str {
    let text = error.to_string();
    if text.contains("NotEnoughGas") {
        "OOG"
    } else if text.contains("Trap") {
        "TRAP"
    } else {
        "FAIL"
    }
}

fn run_capture(command: &Path, args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new(command)
        .current_dir(cwd)
        .args(args)
        .output()
        .with_context(|| format!("running {}", command.display()))?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            command.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}
