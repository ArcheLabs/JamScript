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
    gas: i64,
    allocation_count: u64,
    requested_bytes: u64,
    high_water_mark: u64,
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
    let report = run(&engine, &blob, 3)?;
    fs::write(output.join("m1.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("Node: {}", node_version.trim());
    println!(
        "control-flow PVM: PASS gas={} alloc={} requested={} high_water={}",
        report.gas, report.allocation_count, report.requested_bytes, report.high_water_mark
    );
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
        gas: GAS_LIMIT - instance.gas(),
        allocation_count: read_metric(&mut instance, "probe_allocation_count")?,
        requested_bytes: read_metric(&mut instance, "probe_requested_bytes")?,
        high_water_mark: read_metric(&mut instance, "probe_high_water_mark")?,
    })
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
