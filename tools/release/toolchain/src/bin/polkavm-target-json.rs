use anyhow::{Context, Result};
use polkavm_linker::{target_json_path, RustcVersion, TargetJsonArgs};

fn main() -> Result<()> {
    let mut args = TargetJsonArgs::default();
    args.rustc_version = RustcVersion::Rustc_1_91;
    args.is_64_bit = true;
    let path = target_json_path(args)
    .map_err(|error| anyhow::anyhow!(error))
    .context("resolve the locked PolkaVM target JSON")?;
    println!("{}", path.display());
    Ok(())
}
