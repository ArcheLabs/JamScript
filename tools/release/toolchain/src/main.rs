use jamscript_toolchain::{sha256_file, DistributionManifest, ToolchainManager};
use std::{env, fs, path::Path};

fn main() -> anyhow::Result<()> {
    let mut args = env::args_os().skip(1);
    let archive = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("archive path is required"))?;
    let cache = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("cache path is required"))?;
    let manifest_path = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("distribution manifest path is required"))?;
    let archive = Path::new(&archive);
    let cache = Path::new(&cache);
    let mut manifest: DistributionManifest =
        toml::from_str(&fs::read_to_string(manifest_path)?)?;
    let bundle = manifest
        .platforms
        .get_mut("linux-x86_64")
        .ok_or_else(|| anyhow::anyhow!("linux-x86_64 bundle is missing"))?;
    bundle.url = format!("file://{}", archive.display());
    bundle.sha256 = sha256_file(archive)?;
    bundle.size = fs::metadata(archive)?.len();
    bundle.published = true;
    let candidate = toml::to_string(&manifest)?;
    let manager = ToolchainManager::from_manifest_str(&candidate, Path::new("/candidate-source"))?
        .with_cache_home(cache);
    let installed = manager.install()?;
    println!("{}", installed.root.display());
    Ok(())
}
