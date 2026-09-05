use anyhow::{Context, Result};
use jamscript_target_jam::link_elf_to_jam;
use std::{env, path::PathBuf};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let elf = PathBuf::from(args.next().ok_or_else(|| {
        anyhow::anyhow!("usage: jam-target-convert <input.elf> <output.blob> [output.polkavm]")
    })?);
    let blob = PathBuf::from(args.next().ok_or_else(|| {
        anyhow::anyhow!("usage: jam-target-convert <input.elf> <output.blob> [output.polkavm]")
    })?);
    let polkavm = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| blob.with_extension("polkavm"));
    if args.next().is_some() {
        anyhow::bail!("usage: jam-target-convert <input.elf> <output.blob> [output.polkavm]");
    }
    link_elf_to_jam(&elf, &blob, &polkavm).with_context(|| format!("converting {}", elf.display()))
}
