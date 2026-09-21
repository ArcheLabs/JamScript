use anyhow::{Context, Result};
use jamscript_target_jam::link_elf_to_jam;
use service_build_polkavm::{
    NativeArchive, PolkaVmBuildConfig, PolkaVmBuildRequest, PolkaVmBuilder,
};
use service_runtime_core::blake2_256;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::tempdir;

fn main() -> Result<()> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/ownership-control-service"));
    fs::create_dir_all(&output)
        .with_context(|| format!("creating output directory {}", output.display()))?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/jamscript-ownership-control-service/guest/Cargo.toml")
        .canonicalize()
        .context("locating Ownership Control guest manifest")?;
    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/jamscript-target-jam/sdk")
        .canonicalize()
        .context("locating JAM SDK")?;
    let native_work = tempdir().context("creating JAM SDK build directory")?;
    let native_archive = compile_jam_archive(
        &sdk,
        native_work.path(),
        env::var_os("JAMSCRIPT_CLANG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/clang")),
        env::var_os("JAMSCRIPT_LLVM_AR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/lib/llvm-20/bin/llvm-ar")),
    )?;
    let artifacts = PolkaVmBuilder::new(PolkaVmBuildConfig {
        rustflags: Some("-C link-arg=-z -C link-arg=notext".into()),
        ..Default::default()
    })
    .build(&PolkaVmBuildRequest {
        manifest_path: manifest,
        output_dir: output.clone(),
        native_archives: vec![native_archive],
        required_exports: vec![
            "minijam_refine".into(),
            "minijam_accumulate".into(),
            "jamscript_plan_v1".into(),
            "jamscript_backend_metadata_v1".into(),
        ],
        require_relocations: true,
    })?;
    let blob = output.join("service.blob");
    let pvm = output.join("service.pvm");
    link_elf_to_jam(&artifacts.elf, &blob, &pvm).context("linking Ownership Control service")?;
    let metadata = format!(
        "{{\n  \"serviceKey\": \"0x{}\",\n  \"elfBlake2\": \"0x{}\",\n  \"blobBlake2\": \"0x{}\",\n  \"pvmBlake2\": \"0x{}\",\n  \"targetEnvironment\": \"polkavm\",\n  \"exports\": [\"minijam_refine\", \"minijam_accumulate\", \"jamscript_plan_v1\", \"jamscript_backend_metadata_v1\"]\n}}\n",
        hex(jamscript_ownership_control_service::OWNERSHIP_CONTROL_SERVICE_KEY_V1.as_bytes()),
        hex(&blake2_256(&fs::read(&artifacts.elf)?)),
        hex(&blake2_256(&fs::read(&blob)?)),
        hex(&blake2_256(&fs::read(&pvm)?)),
    );
    fs::write(output.join("build.json"), metadata).context("writing build provenance")?;
    println!("Ownership Control service built in {}", output.display());
    Ok(())
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn compile_jam_archive(
    sdk: &Path,
    work: &Path,
    clang: PathBuf,
    ar: PathBuf,
) -> Result<NativeArchive> {
    let include = sdk.join("include");
    let sources = ["host", "minijam", "crypto"]
        .into_iter()
        .map(|unit| sdk.join("src").join(format!("{unit}.c")))
        .collect::<Vec<_>>();
    let objects = sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let object = work.join(format!("jam_target_guest_{index}.o"));
            let status = Command::new(&clang)
                .args([
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
                    "-std=c11",
                ])
                .arg("-I")
                .arg(&include)
                .args(["-c", source.to_str().unwrap(), "-o"])
                .arg(&object)
                .status()
                .with_context(|| format!("running {}", clang.display()))?;
            if !status.success() {
                anyhow::bail!("compiling JAM SDK source failed: {}", source.display());
            }
            Ok(object)
        })
        .collect::<Result<Vec<_>>>()?;
    let archive = work.join("libjam_target_guest.a");
    let status = Command::new(&ar)
        .arg("crs")
        .arg(&archive)
        .args(&objects)
        .status()
        .with_context(|| format!("running {}", ar.display()))?;
    if !status.success() {
        anyhow::bail!("archiving JAM SDK failed");
    }
    Ok(NativeArchive {
        name: "jam_target_guest".into(),
        path: archive,
    })
}
