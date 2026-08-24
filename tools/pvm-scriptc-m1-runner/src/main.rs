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
struct Report {
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
    let node_version = capture(&node, &["--version"], &scriptc)?;
    if node_version.trim() != "v24.15.0" {
        bail!(
            "M1 requires pinned Node v24.15.0, got {}",
            node_version.trim()
        );
    }
    let output = repo.join("target/pvm-scriptc-m1");
    fs::create_dir_all(&output)?;
    let status = Command::new(&node)
        .current_dir(&scriptc)
        .env("SCRIPTC_M1_OUT", &output)
        .arg("m1/conformance.mjs")
        .status()?;
    if !status.success() {
        bail!("ScriptC M1 conformance generation failed");
    }
    let runtime = scriptc.join("node_modules/@scriptc/runtime");
    let generated = output.join("conformance.lib.c");
    if !generated.is_file() || !runtime.is_dir() {
        bail!("M1 generated C or runtime is missing");
    }
    env::set_var("SCRIPTC_M06_SCALAR_C", &generated);
    env::set_var("SCRIPTC_M06_RUNTIME", &runtime);
    env::set_var(
        "SCRIPTC_M06_EXTRA_C",
        scriptc.join("m1/shims/freestanding.c"),
    );
    let artifacts = PolkaVmBuilder::new(PolkaVmBuildConfig {
        diagnostic: true,
        rustflags: Some("-C link-arg=-nostdlib -C link-arg=--gc-sections".into()),
        ..Default::default()
    })
    .build(&PolkaVmBuildRequest {
        manifest_path: repo.join("tools/pvm-scriptc-m0-guest/Cargo.toml"),
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
    .map_err(|error| anyhow::anyhow!("converting M1 ELF to PVM: {error:?}"))?;
    fs::write(output.join("conformance.pvm"), &blob)?;
    let engine = Engine::new(&{
        let mut config = polkavm::Config::new();
        config.set_backend(Some(polkavm::BackendKind::Interpreter));
        config
    })?;
    let mut reports = Vec::new();
    for stage in 1..=8 {
        match run(&engine, &blob, stage) {
            Ok(report) => reports.push(report),
            Err(error) => reports.push(Report {
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
    fs::write(output.join("m1.json"), serde_json::to_vec_pretty(&reports)?)?;
    println!("Node: {}", node_version.trim());
    println!(
        "M1 PVM stages: PASS max_gas={} max_high_water={}",
        reports
            .iter()
            .filter_map(|report| report.gas)
            .max()
            .unwrap_or_default(),
        reports
            .iter()
            .filter_map(|report| report.high_water_mark)
            .max()
            .unwrap_or_default()
    );
    for report in &reports {
        println!(
            "stage {}: {} gas={} alloc={} requested={} high_water={}{}",
            report.stage,
            report.result,
            report
                .gas
                .map_or_else(|| "-".into(), |value| value.to_string()),
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
                .map_or_else(String::new, |value| format!(" error={value}")),
        );
    }
    if reports.iter().any(|report| report.result != "PASS") {
        bail!("ScriptC M1 PVM stage failed");
    }
    Ok(())
}

fn run(engine: &Engine, blob: &[u8], stage: u64) -> Result<Report> {
    let mut config = ModuleConfig::new();
    config.set_gas_metering(Some(polkavm::GasMeteringKind::Sync));
    let module = Module::new(engine, &config, blob.to_vec().into())?;
    let linker: Linker<(), core::convert::Infallible> = Linker::new();
    let pre = linker.instantiate_pre(&module)?;
    let mut instance = pre.instantiate()?;
    instance.set_gas(GAS_LIMIT);
    let value = instance
        .call_typed_and_get_result::<u64, _>(&mut (), "probe_entry", (stage,))
        .map_err(|error| anyhow::anyhow!("PVM execution: {error:?}"))?;
    if value != 1 {
        bail!("control-flow probe returned {value}");
    }
    Ok(Report {
        stage,
        result: "PASS",
        gas: Some(GAS_LIMIT - instance.gas()),
        allocation_count: Some(read_metric(&mut instance, "probe_allocation_count")?),
        requested_bytes: Some(read_metric(&mut instance, "probe_requested_bytes")?),
        high_water_mark: Some(read_metric(&mut instance, "probe_high_water_mark")?),
        error: None,
    })
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

fn read_metric(
    instance: &mut polkavm::Instance<(), core::convert::Infallible>,
    export: &str,
) -> Result<u64> {
    instance
        .call_typed_and_get_result(&mut (), export, ())
        .map_err(|error| anyhow::anyhow!("reading {export}: {error:?}"))
}

fn capture(command: &Path, args: &[&str], cwd: &Path) -> Result<String> {
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
