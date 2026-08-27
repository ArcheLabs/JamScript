use anyhow::{bail, Context, Result};
use blake2b_simd::Params;
use jamscript_ir::{ActionBodyIr, ServiceIr};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const BACKEND_VERSION: &str = "scriptc-m1";
pub const RUNTIME_PROFILE_VERSION: &str = "scriptc-deterministic-v1";
pub const SCRIPT_C_VERSION: &str = "0.0.34";
pub const TYPESCRIPT_VERSION: &str = "7.0.2";
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Serialize)]
pub struct ScriptcBuildMetadata {
    pub backend: String,
    pub scriptc_version: String,
    pub scriptc_revision: String,
    pub node_version: String,
    pub typescript_version: String,
    pub surface_manifest_hash: String,
    pub package_lock_hash: String,
    pub runtime_profile_version: String,
    pub generated_symbol: String,
}

#[derive(Clone, Debug)]
pub struct ScriptcArtifact {
    pub generated_c: PathBuf,
    pub adapter_c: PathBuf,
    pub metadata: ScriptcBuildMetadata,
}

pub struct ScriptcCompiler {
    pub toolchain_root: PathBuf,
    pub node: PathBuf,
    node_version: String,
}

impl ScriptcCompiler {
    pub fn from_toolchain(toolchain_root: impl Into<PathBuf>) -> Result<Self> {
        let toolchain_root = toolchain_root.into();
        let node = std::env::var_os("SCRIPTC_NODE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("node"));
        let version = command_output(&node, &["--version"], &toolchain_root)?;
        let pinned_node = read_trim(&toolchain_root.join("NODE_VERSION"))?;
        let actual_node = version.trim().trim_start_matches('v');
        if actual_node != pinned_node {
            bail!(
                "ScriptC M1 requires pinned Node {}, got {}",
                pinned_node,
                version.trim()
            );
        }
        Ok(Self {
            toolchain_root,
            node,
            node_version: actual_node.into(),
        })
    }

    pub fn compile_service_action(
        &self,
        ir: &ServiceIr,
        output_dir: &Path,
    ) -> Result<ScriptcArtifact> {
        let action = ir
            .actions
            .first()
            .context("ScriptC service has no action")?;
        let ActionBodyIr::ScriptC {
            symbol,
            source_unit,
            ..
        } = &action.body
        else {
            bail!("ScriptC backend requires a ScriptC action body");
        };
        if source_unit != "service.ts" {
            bail!("unsupported ScriptC source unit `{source_unit}`");
        }
        fs::create_dir_all(output_dir)?;
        let output_dir = fs::canonicalize(output_dir)
            .with_context(|| format!("canonicalizing {}", output_dir.display()))?;
        let source_path = output_dir.join("scriptc_action.ts");
        let spec_path = output_dir.join("scriptc_action.json");
        fs::write(&source_path, &ir.source)?;
        fs::write(
            &spec_path,
            serde_json::json!({
                "source": source_path,
                "action": symbol,
                "input_fields": action.input.iter().map(|field| {
                    serde_json::json!({ "name": field.name, "type": field.ty.abi_name() })
                }).collect::<Vec<_>>(),
                "output": output_dir,
            })
            .to_string(),
        )?;
        verify_surface_manifest(&self.toolchain_root)?;
        let script = self.toolchain_root.join("m1/compile-action.mjs");
        let status = Command::new(&self.node)
            .current_dir(&self.toolchain_root)
            .arg(script)
            .arg(&spec_path)
            .status()
            .context("starting ScriptC M1 compiler")?;
        if !status.success() {
            bail!("ScriptC failed to compile action `{symbol}`");
        }
        let generated_c = [
            output_dir.join("scriptc_action.lib.c"),
            output_dir.join("scriptc_action.transformed.lib.c"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| format!("ScriptC did not emit C for action `{symbol}`"))?;
        if generated_c.file_name().and_then(|name| name.to_str()) != Some("scriptc_action.lib.c") {
            fs::copy(&generated_c, output_dir.join("scriptc_action.lib.c"))?;
        }
        let compiler_version = read_json_string(
            &self
                .toolchain_root
                .join("node_modules/@scriptc/compiler/package.json"),
            "version",
        )?;
        if compiler_version != SCRIPT_C_VERSION {
            bail!(
                "ScriptC compiler version mismatch: pinned {}, installed {}",
                SCRIPT_C_VERSION,
                compiler_version
            );
        }
        let typescript_version = read_json_string(
            &self
                .toolchain_root
                .join("node_modules/typescript/package.json"),
            "version",
        )?;
        if typescript_version != TYPESCRIPT_VERSION {
            bail!(
                "TypeScript version mismatch: pinned {}, installed {}",
                TYPESCRIPT_VERSION,
                typescript_version
            );
        }
        let adapter_c = output_dir.join("scriptc_action_adapter.c");
        let entry_symbol = format!("jamscript_scriptc_{symbol}_entry");
        let stable_symbol = format!("jamscript_scriptc_{symbol}_v1");
        fs::write(
            &adapter_c,
            adapter_source(symbol, &entry_symbol, &stable_symbol),
        )?;
        let metadata = ScriptcBuildMetadata {
            backend: BACKEND_VERSION.into(),
            scriptc_version: SCRIPT_C_VERSION.into(),
            scriptc_revision: read_revision(&self.toolchain_root.join("REVISION"))?,
            node_version: self.node_version.clone(),
            typescript_version,
            surface_manifest_hash: sha256_file(
                &self
                    .toolchain_root
                    .join("node_modules/@scriptc/compiler/surface-manifest.json"),
            )?,
            package_lock_hash: hash_file(&self.toolchain_root.join("package-lock.json"))?,
            runtime_profile_version: RUNTIME_PROFILE_VERSION.into(),
            generated_symbol: stable_symbol,
        };
        Ok(ScriptcArtifact {
            generated_c,
            adapter_c,
            metadata,
        })
    }
}

fn adapter_source(symbol: &str, entry_symbol: &str, stable_symbol: &str) -> String {
    format!(
        "#include <stdint.h>\n#include <stddef.h>\nextern void jamscript_scriptc_{symbol}_init(void);\nextern double {entry_symbol}(double);\nuint32_t {stable_symbol}(const uint8_t *input, uint32_t input_len, uint64_t *output) {{ if (input_len != 8 || input == NULL || output == NULL) return 5; uint64_t raw = 0; for (uint32_t i = 0; i < 8; i++) raw |= ((uint64_t)input[i]) << (8 * i); if (raw > {MAX_SAFE_INTEGER}ULL) return 6; jamscript_scriptc_{symbol}_init(); double value = {entry_symbol}((double)raw); if (!(value >= 0.0) || value > {MAX_SAFE_INTEGER}.0) return 7; *output = (uint64_t)value; return 0; }}\n"
    )
}

fn command_output(command: &Path, args: &[&str], cwd: &Path) -> Result<String> {
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

fn read_trim(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)?.trim().into())
}

fn read_revision(path: &Path) -> Result<String> {
    let contents = read_trim(path)?;
    Ok(contents
        .lines()
        .find_map(|line| line.strip_prefix("commit="))
        .unwrap_or(contents.as_str())
        .to_owned())
}

fn read_json_string(path: &Path, field: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("{} is missing string field `{field}`", path.display()))
}

fn hash_file(path: &Path) -> Result<String> {
    let hash = Params::new().hash_length(32).hash(&fs::read(path)?);
    Ok(format!(
        "0x{}",
        hash.as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

fn sha256_file(path: &Path) -> Result<String> {
    let hash = Sha256::digest(fs::read(path)?);
    Ok(format!(
        "0x{}",
        hash.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn verify_surface_manifest(toolchain_root: &Path) -> Result<()> {
    let lock_path = toolchain_root.join("SURFACE_MANIFEST.json");
    let lock: serde_json::Value = serde_json::from_slice(&fs::read(&lock_path)?)?;
    let expected = lock
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .context("SURFACE_MANIFEST.json is missing sha256")?;
    let manifest_path = toolchain_root.join("node_modules/@scriptc/compiler/surface-manifest.json");
    let actual = Sha256::digest(fs::read(&manifest_path)?);
    let actual = actual
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        bail!(
            "ScriptC surface manifest hash mismatch: expected {}, got {}",
            expected,
            actual
        );
    }
    Ok(())
}

#[cfg(test)]
mod boundary_tests {
    use super::{adapter_source, MAX_SAFE_INTEGER};

    #[test]
    fn u64_adapter_accepts_only_exact_safe_integer_domain() {
        for value in [0, 1, 1u64 << 32, MAX_SAFE_INTEGER] {
            assert!(value <= MAX_SAFE_INTEGER);
        }
        for value in [MAX_SAFE_INTEGER + 1, u64::MAX] {
            assert!(value > MAX_SAFE_INTEGER);
        }
    }

    #[test]
    fn generated_adapter_contains_safe_integer_guards() {
        let source = adapter_source(
            "increment",
            "jamscript_scriptc_increment_entry",
            "jamscript_scriptc_increment_v1",
        );
        assert!(source.contains("raw > 9007199254740991ULL"));
        assert!(source.contains("value > 9007199254740991.0"));
        assert!(source.contains("input_len != 8"));
    }
}
