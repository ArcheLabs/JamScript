use std::{env, fs::File, io, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("output archive path is required"))?;
    let stdout = io::stdin();
    let mut input = stdout.lock();
    let file = File::create(&output)?;
    let mut encoder = zstd::Encoder::new(file, 19)?;
    io::copy(&mut input, &mut encoder)?;
    encoder.finish()?.sync_all().map_err(anyhow::Error::from)
}
