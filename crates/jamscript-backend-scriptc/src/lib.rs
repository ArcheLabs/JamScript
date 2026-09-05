use anyhow::{bail, Context, Result};
use blake2b_simd::Params;
use jamscript_ir::{action_selector, ActionBodyIr, ServiceIr};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const BACKEND_VERSION: &str = "scriptc-m2";
pub const RUNTIME_PROFILE_VERSION: &str = "scriptc-deterministic-v1";
pub const SCRIPT_C_VERSION: &str = "0.0.34";
pub const TYPESCRIPT_VERSION: &str = "7.0.2";

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
    pub generated_actions: Vec<ScriptcGeneratedAction>,
    #[serde(rename = "typedRuntimeVersion")]
    pub typed_runtime_version: u8,
    #[serde(rename = "stateViewVersion")]
    pub state_view_version: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScriptcGeneratedAction {
    pub name: String,
    pub selector: String,
    pub symbol: String,
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
        Self::from_paths(toolchain_root, node)
    }

    pub fn from_paths(
        toolchain_root: impl Into<PathBuf>,
        node: impl Into<PathBuf>,
    ) -> Result<Self> {
        let toolchain_root = toolchain_root.into();
        let node = node.into();
        let version = command_output(&node, &["--version"], &toolchain_root)?;
        let pinned_node = read_trim(&toolchain_root.join("NODE_VERSION"))?;
        let actual_node = version.trim().trim_start_matches('v');
        if actual_node != pinned_node {
            bail!(
                "ScriptC M2 requires pinned Node {}, got {}",
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

    pub fn compile_service(&self, ir: &ServiceIr, output_dir: &Path) -> Result<ScriptcArtifact> {
        if ir.actions.is_empty() {
            bail!("ScriptC service has no action");
        }
        for action in &ir.actions {
            let ActionBodyIr::ScriptC { source_unit, .. } = &action.body else {
                bail!("ScriptC backend requires every action to have a ScriptC body");
            };
            if source_unit != "service.ts" {
                bail!("unsupported ScriptC source unit `{source_unit}`");
            }
        }
        fs::create_dir_all(output_dir)?;
        let output_dir = fs::canonicalize(output_dir)
            .with_context(|| format!("canonicalizing {}", output_dir.display()))?;
        let source_path = output_dir.join("scriptc_service.ts");
        let spec_path = output_dir.join("scriptc_service.json");
        fs::write(&source_path, &ir.source)?;
        fs::write(
            &spec_path,
            serde_json::json!({
                "source": source_path,
                "package_name": ir.package_name,
                "states": ir.states,
                "actions": ir.actions.iter().map(|action| serde_json::json!({
                    "name": action.name,
                    "auth": action.auth,
                    "input": action.input,
                })).collect::<Vec<_>>(),
                "queries": ir.queries,
                "output": output_dir,
            })
            .to_string(),
        )?;
        verify_surface_manifest(&self.toolchain_root)?;
        let script = self.toolchain_root.join("m2/compile-service.mjs");
        let mut command = Command::new(&self.node);
        command
            .current_dir(&self.toolchain_root)
            .arg(script)
            .arg(&spec_path);
        let managed_bin = self.toolchain_root.parent().map(|root| root.join("bin"));
        if let Some(managed_bin) = managed_bin.filter(|path| path.is_dir()) {
            let mut path_entries = vec![managed_bin];
            if let Some(path) = env::var_os("PATH") {
                path_entries.extend(env::split_paths(&path));
            }
            command.env(
                "PATH",
                env::join_paths(path_entries).context("constructing ScriptC managed PATH")?,
            );
        }
        let status = command.status().context("starting ScriptC M2 compiler")?;
        if !status.success() {
            bail!("ScriptC failed to compile service `{}`", ir.package_name);
        }
        let generated_c = [
            output_dir.join("scriptc_service.lib.c"),
            output_dir.join("scriptc_service.transformed.lib.c"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| format!("ScriptC did not emit C for service `{}`", ir.package_name))?;
        if generated_c.file_name().and_then(|name| name.to_str()) != Some("scriptc_service.lib.c") {
            fs::copy(&generated_c, output_dir.join("scriptc_service.lib.c"))?;
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
        let adapter_c = output_dir.join("scriptc_service_adapter.c");
        fs::write(
            &adapter_c,
            "/* ScriptC M2 exports its canonical bytes ABI directly. */\n",
        )?;
        let generated_actions = ir
            .actions
            .iter()
            .map(|action| ScriptcGeneratedAction {
                name: action.name.clone(),
                selector: format!(
                    "0x{}",
                    action_selector(&action.name)
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                ),
                symbol: format!("jamscript_scriptc_{}_entry_v1", action.name),
            })
            .collect();
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
            generated_actions,
            typed_runtime_version: 1,
            state_view_version: 1,
        };
        Ok(ScriptcArtifact {
            generated_c,
            adapter_c,
            metadata,
        })
    }
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
