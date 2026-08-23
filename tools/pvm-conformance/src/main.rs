use anyhow::{bail, Context, Result};
use polkavm::{Engine, Linker, Module, ModuleConfig};
use polkavm_linker::{program_from_elf, Config as LinkConfig, TargetInstructionSet};
use serde::Serialize;
use service_build_polkavm::{NativeArchive, PolkaVmBuildConfig, PolkaVmBuildRequest, PolkaVmBuilder};
use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const GAS_LIMIT: i64 = 5_000_000;

#[derive(Debug, Serialize)]
struct ProbeReport {
    probe: String,
    result: String,
    gas: Option<i64>,
    failure_stage: Option<String>,
    failure_reason: Option<String>,
}

struct Probe<'a> {
    name: &'a str,
    manifest: &'a str,
    stages: &'a [&'a str],
    native: bool,
}

fn main() -> Result<()> {
    let repo = repo_root();
    let output = repo.join("target/pvm-conformance");
    fs::create_dir_all(&output)?;
    let probes = [
        Probe {
            name: "ed25519",
            manifest: "tools/pvm-ed25519-probe/Cargo.toml",
            stages: &["PublicKey decode", "Signature decode", "Ed25519 verify"],
            native: false,
        },
        Probe {
            name: "sr25519",
            manifest: "tools/pvm-crypto-probe/Cargo.toml",
            stages: &["PublicKey decode", "Signature decode", "Merlin transcript", "sr25519 verify"],
            native: false,
        },
        Probe {
            name: "sp-trie",
            manifest: "tools/pvm-managed-state-probe/Cargo.toml",
            stages: &["ProofState from witness", "trie get", "transactional set", "finish root"],
            native: false,
        },
        Probe {
            name: "Rust+C",
            manifest: "tools/pvm-native-c-probe/Cargo.toml",
            stages: &["Rust entry", "Rust -> C -> Rust"],
            native: true,
        },
    ];

    let engine = make_engine()?;
    let mut reports = Vec::new();
    for probe in probes {
        reports.push(build_and_run(&repo, &output, &engine, probe)?);
    }
    fs::write(
        output.join("conformance.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;

    println!("{:<18} {:<8} {:<12} Failure stage", "Probe", "Result", "Gas");
    for report in &reports {
        println!(
            "{:<18} {:<8} {:<12} {}",
            report.probe,
            report.result,
            report.gas.map_or_else(|| "-".into(), |gas| gas.to_string()),
            report.failure_stage.as_deref().unwrap_or("-")
        );
    }
    if reports.iter().any(|report| report.result != "PASS") {
        bail!("one or more PolkaVM conformance probes failed");
    }
    Ok(())
}

fn build_and_run(repo: &Path, output: &Path, engine: &Engine, probe: Probe<'_>) -> Result<ProbeReport> {
    let manifest = repo.join(probe.manifest);
    let probe_output = output.join(probe.name.replace('+', "-"));
    fs::create_dir_all(&probe_output)?;

    let native_archives = if probe.native {
        vec![compile_native_archive(repo, &probe_output)?]
    } else {
        Vec::new()
    };
    let artifacts = PolkaVmBuilder::new(PolkaVmBuildConfig::default()).build(&PolkaVmBuildRequest {
        manifest_path: manifest,
        output_dir: probe_output.clone(),
        native_archives,
        required_exports: vec!["probe_entry".into()],
        require_relocations: false,
    })?;

    let elf = fs::read(&artifacts.elf)?;
    let mut link_config = LinkConfig::default();
    link_config.set_strip(true);
    let blob = program_from_elf(link_config, TargetInstructionSet::JamV1, &elf)
        .map_err(|error| anyhow::anyhow!("converting {} ELF to a PVM blob: {error:?}", probe.name))?;
    fs::write(probe_output.join("probe.blob"), &blob)?;
    let mut last_gas = None;
    for (index, stage) in probe.stages.iter().enumerate() {
        match run_stage(engine, &blob, (index + 1) as u64) {
            Ok(gas) => last_gas = Some(gas),
            Err(error) => {
                return Ok(ProbeReport {
                    probe: probe.name.into(),
                    result: classify_failure(&error),
                    gas: last_gas,
                    failure_stage: Some((*stage).into()),
                    failure_reason: Some(error.to_string()),
                });
            }
        }
    }
    Ok(ProbeReport {
        probe: probe.name.into(),
        result: "PASS".into(),
        gas: last_gas,
        failure_stage: None,
        failure_reason: None,
    })
}

fn run_stage(engine: &Engine, blob: &[u8], stage: u64) -> Result<i64> {
    let module = Module::new(engine, &module_config(), blob.to_vec().into())?;
    let linker: Linker<(), core::convert::Infallible> = Linker::new();
    let pre = linker.instantiate_pre(&module)?;
    let mut instance = pre.instantiate()?;
    instance.set_gas(GAS_LIMIT);
    let result = instance
        .call_typed_and_get_result::<u64, _>(&mut (), "probe_entry", (stage,))
        .map_err(|error| anyhow::anyhow!("PVM execution: {error:?}"))?;
    if result != 1 {
        bail!("probe returned {result}");
    }
    Ok(GAS_LIMIT - instance.gas())
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

fn compile_native_archive(repo: &Path, output: &Path) -> Result<NativeArchive> {
    let clang = env::var_os("JAMSCRIPT_CLANG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/clang"));
    let llvm_ar = env::var_os("JAMSCRIPT_LLVM_AR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/llvm-ar"));
    let source = repo.join("tools/pvm-native-c-probe/native.c");
    let object = output.join("native.o");
    let archive = output.join("libnative_probe.a");
    let compile = Command::new(&clang)
        .args([
            "--target=riscv64-unknown-elf",
            "-march=rv64emac",
            "-mabi=lp64e",
            "-ffreestanding",
            "-fno-builtin",
            "-fPIC",
            "-O2",
            "-c",
            source.to_str().unwrap(),
            "-o",
            object.to_str().unwrap(),
        ])
        .output()
        .with_context(|| format!("starting {}", clang.display()))?;
    if !compile.status.success() {
        bail!("native C compile failed: {}", String::from_utf8_lossy(&compile.stderr));
    }
    let archive_run = Command::new(&llvm_ar)
        .args(["rcs", archive.to_str().unwrap(), object.to_str().unwrap()])
        .output()
        .with_context(|| format!("starting {}", llvm_ar.display()))?;
    if !archive_run.status.success() {
        bail!("native archive failed: {}", String::from_utf8_lossy(&archive_run.stderr));
    }
    Ok(NativeArchive {
        name: "native_probe".into(),
        path: archive,
    })
}

fn classify_failure(error: &anyhow::Error) -> String {
    let text = error.to_string();
    if text.contains("NotEnoughGas") {
        "OOG".into()
    } else if text.contains("Trap") {
        "TRAP".into()
    } else {
        "FAIL".into()
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}
