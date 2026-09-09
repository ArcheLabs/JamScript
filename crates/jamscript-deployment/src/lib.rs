//! Backend-neutral JamScript deployment.
//!
//! The CLI owns user interaction. This crate owns named network resolution,
//! artifact preflight, transport, backend mapping, result normalization, and
//! deployment records. MiniJAM is one backend, not JamScript's global network.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use blake2b_simd::Params as Blake2Params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use url::Url;

pub const DEFAULT_MIN_ITEM_GAS: u64 = 5_000_000;
pub const DEFAULT_MIN_MEMO_GAS: u64 = 1_000_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkKind {
    #[serde(rename = "minijam")]
    MiniJam,
    #[serde(rename = "jam")]
    Jam,
}

impl NetworkKind {
    pub fn parse(value: &str) -> Result<Self, DeploymentError> {
        match value {
            "minijam" => Ok(Self::MiniJam),
            "jam" => Ok(Self::Jam),
            other => Err(DeploymentError::new(
                ErrorCode::UnsupportedNetworkKind,
                format!("unsupported deployment backend '{other}'; supported backends: minijam"),
            )),
        }
    }
}

impl fmt::Display for NetworkKind {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::MiniJam => "minijam",
            Self::Jam => "jam",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    pub kind: String,
    pub deployment_rpc: Option<String>,
    pub node_rpc: Option<String>,
    pub backend_rpc: Option<String>,
    pub genesis_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConfig {
    pub default_network: Option<String>,
}

pub type NetworkTable = BTreeMap<String, NetworkConfig>;

#[derive(Clone, Debug, Default)]
pub struct NetworkOverrides {
    pub network: Option<String>,
    pub kind: Option<String>,
    pub deployment_rpc: Option<String>,
    pub node_rpc: Option<String>,
    pub backend_rpc: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedNetwork {
    pub name: Option<String>,
    pub kind: NetworkKind,
    pub deployment_rpc: String,
    pub node_rpc: Option<String>,
    pub backend_rpc: Option<String>,
    pub genesis_hash: Option<String>,
    pub genesis_pinned: bool,
}

impl ResolvedNetwork {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("custom")
    }
}

pub fn validate_networks(
    networks: Option<&NetworkTable>,
    deployment: Option<&DeploymentConfig>,
) -> Result<(), DeploymentError> {
    let networks = networks.cloned().unwrap_or_default();
    for (name, config) in &networks {
        validate_network_name(name)?;
        let kind = NetworkKind::parse(&config.kind).map_err(|error| {
            DeploymentError::new(
                ErrorCode::NetworkConfigInvalid,
                format!("network '{name}': {error}"),
            )
        })?;
        match kind {
            NetworkKind::MiniJam => {
                let deployment_rpc = config.deployment_rpc.as_deref().ok_or_else(|| {
                    DeploymentError::new(
                        ErrorCode::NetworkConfigInvalid,
                        format!("network '{name}' with kind 'minijam' requires deployment_rpc"),
                    )
                })?;
                validate_http_url(deployment_rpc, "deployment_rpc")?;
                if let Some(node_rpc) = config.node_rpc.as_deref() {
                    validate_http_url(node_rpc, "node_rpc")?;
                }
                if let Some(backend_rpc) = config.backend_rpc.as_deref() {
                    validate_http_url(backend_rpc, "backend_rpc")?;
                }
                if let Some(genesis_hash) = config.genesis_hash.as_deref() {
                    normalize_hash(genesis_hash).map_err(|error| {
                        DeploymentError::new(
                            ErrorCode::NetworkConfigInvalid,
                            format!("network '{name}' genesis_hash: {error}"),
                        )
                    })?;
                    if config.node_rpc.is_none() {
                        return Err(DeploymentError::new(
                            ErrorCode::NetworkConfigInvalid,
                            format!("network '{name}' pins genesis_hash but has no node_rpc"),
                        ));
                    }
                }
            }
            NetworkKind::Jam => {
                // JAM-specific fields remain intentionally open in v0.1.
                if let Some(value) = config.deployment_rpc.as_deref() {
                    validate_http_url(value, "deployment_rpc")?;
                }
                if let Some(value) = config.node_rpc.as_deref() {
                    validate_http_url(value, "node_rpc")?;
                }
            }
        }
    }
    if let Some(default_network) = deployment.and_then(|value| value.default_network.as_deref()) {
        if !networks.contains_key(default_network) {
            return Err(DeploymentError::new(
                ErrorCode::NetworkConfigInvalid,
                format!(
                    "deployment.default_network references unknown network '{default_network}'"
                ),
            ));
        }
    }
    Ok(())
}

pub fn resolve_network(
    networks: Option<&NetworkTable>,
    deployment: Option<&DeploymentConfig>,
    mut overrides: NetworkOverrides,
) -> Result<ResolvedNetwork, DeploymentError> {
    validate_networks(networks, deployment)?;
    let table = networks.cloned().unwrap_or_default();
    if overrides.network.is_none() {
        overrides.network = std::env::var("JAMSCRIPT_NETWORK").ok();
    }
    if overrides.deployment_rpc.is_none() {
        overrides.deployment_rpc = std::env::var("JAMSCRIPT_DEPLOYMENT_RPC").ok();
    }
    if overrides.node_rpc.is_none() {
        overrides.node_rpc = std::env::var("JAMSCRIPT_NODE_RPC").ok();
    }
    if overrides.backend_rpc.is_none() {
        overrides.backend_rpc = std::env::var("JAMSCRIPT_BACKEND_RPC").ok();
    }
    if overrides.kind.is_none() {
        overrides.kind = std::env::var("JAMSCRIPT_NETWORK_KIND").ok();
    }

    let selected_name = overrides
        .network
        .clone()
        .or_else(|| deployment.and_then(|value| value.default_network.clone()));
    let (name, base) = match selected_name {
        Some(name) => {
            let base = table.get(&name).ok_or_else(|| {
                DeploymentError::new(
                    ErrorCode::NetworkNotFound,
                    format!("network '{name}' is not configured"),
                )
            })?;
            (Some(name), Some(base))
        }
        None => (None, None),
    };
    let raw_kind = overrides
        .kind
        .as_deref()
        .or_else(|| base.map(|value| value.kind.as_str()))
        .ok_or_else(|| {
            let configured = table.keys().cloned().collect::<Vec<_>>().join("  ");
            let suffix = if configured.is_empty() {
                String::new()
            } else {
                format!("\nConfigured networks:\n  {configured}")
            };
            DeploymentError::new(
                ErrorCode::NetworkNotFound,
                format!(
                    "No deployment network selected.{suffix}\n\nUse: jams deploy --network <name>"
                ),
            )
        })?;
    let kind = NetworkKind::parse(raw_kind)?;
    let deployment_rpc = overrides
        .deployment_rpc
        .clone()
        .or_else(|| base.and_then(|value| value.deployment_rpc.clone()));
    let node_rpc = overrides
        .node_rpc
        .clone()
        .or_else(|| base.and_then(|value| value.node_rpc.clone()));
    let backend_rpc = overrides
        .backend_rpc
        .clone()
        .or_else(|| base.and_then(|value| value.backend_rpc.clone()));
    let genesis_hash = base
        .and_then(|value| value.genesis_hash.as_deref())
        .map(normalize_hash)
        .transpose()?
        .map(|value| hash_hex(&value));

    match kind {
        NetworkKind::MiniJam => {
            let deployment_rpc = deployment_rpc.ok_or_else(|| {
                DeploymentError::new(
                    ErrorCode::NetworkConfigInvalid,
                    "MiniJAM deployment requires deployment_rpc",
                )
            })?;
            validate_http_url(&deployment_rpc, "deployment_rpc")?;
            if let Some(value) = node_rpc.as_deref() {
                validate_http_url(value, "node_rpc")?;
            }
            if let Some(value) = backend_rpc.as_deref() {
                validate_http_url(value, "backend_rpc")?;
            }
            if genesis_hash.is_some() && node_rpc.is_none() {
                return Err(DeploymentError::new(
                    ErrorCode::NetworkConfigInvalid,
                    "genesis_hash verification requires node_rpc",
                ));
            }
            Ok(ResolvedNetwork {
                name,
                kind,
                deployment_rpc,
                node_rpc,
                backend_rpc,
                genesis_hash,
                genesis_pinned: base.and_then(|value| value.genesis_hash.as_ref()).is_some(),
            })
        }
        NetworkKind::Jam => Err(DeploymentError::new(
            ErrorCode::UnsupportedNetworkKind,
            "JAM deployment is not supported in v0.1; JamScript currently supports: minijam",
        )),
    }
}

pub fn validate_http_url(value: &str, field: &str) -> Result<(), DeploymentError> {
    let parsed = Url::parse(value).map_err(|error| {
        DeploymentError::new(
            ErrorCode::NetworkConfigInvalid,
            format!("{field} is not a valid URL: {error}"),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(DeploymentError::new(
            ErrorCode::NetworkConfigInvalid,
            format!("{field} must use an http or https URL with a host"),
        ));
    }
    Ok(())
}

pub fn redact_url(value: &str) -> String {
    let Ok(mut parsed) = Url::parse(value) else {
        return "<invalid-url>".to_owned();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

fn validate_network_name(value: &str) -> Result<(), DeploymentError> {
    if value.trim().is_empty() || value.chars().any(|character| character.is_control()) {
        return Err(DeploymentError::new(
            ErrorCode::NetworkConfigInvalid,
            "network names must not be empty or contain control characters",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ServiceArtifact {
    pub directory: PathBuf,
    pub blob_path: PathBuf,
    pub blob: Vec<u8>,
    pub code_hash: [u8; 32],
    pub build_id: String,
    pub service_key: Option<[u8; 32]>,
    pub min_item_gas: u64,
    pub min_memo_gas: u64,
}

pub fn load_service_artifact(directory: &Path) -> Result<ServiceArtifact, DeploymentError> {
    let directory = directory.canonicalize().map_err(|error| {
        DeploymentError::new(
            ErrorCode::ArtifactNotFound,
            format!(
                "cannot locate artifact directory {}: {error}",
                directory.display()
            ),
        )
    })?;
    let blob_path = directory.join("service.blob");
    let build_path = directory.join("build.json");
    let checksums_path = directory.join("checksums.json");
    let blob = fs::read(&blob_path).map_err(|error| {
        DeploymentError::new(
            ErrorCode::ArtifactNotFound,
            format!("missing service.blob: {error}"),
        )
    })?;
    let build_bytes = fs::read(&build_path).map_err(|error| {
        DeploymentError::new(
            ErrorCode::ArtifactNotFound,
            format!("missing build.json: {error}"),
        )
    })?;
    verify_checksums(&directory, &checksums_path)?;
    let metadata: serde_json::Value = serde_json::from_slice(&build_bytes).map_err(|error| {
        DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            format!("build.json is not valid JSON: {error}"),
        )
    })?;
    let code_hash = normalize_hash(value_string(&metadata, "code_hash")?.as_str())?;
    let actual_code_hash = blake2_hash(&blob);
    if code_hash != actual_code_hash {
        return Err(DeploymentError::new(
            ErrorCode::ArtifactCodeHashMismatch,
            format!(
                "build.json code_hash {} does not match service.blob {}",
                hash_hex(&code_hash),
                hash_hex(&actual_code_hash)
            ),
        ));
    }
    let min_item_gas = value_u64(&metadata, &["min_item_gas", "minItemGas"])?;
    let min_memo_gas = value_u64(&metadata, &["min_memo_gas", "minMemoGas"])?;
    if min_item_gas == 0 || min_memo_gas == 0 {
        return Err(DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            "build.json deployment gas values must be greater than zero",
        ));
    }
    let service_key = metadata
        .get("serviceKey")
        .and_then(serde_json::Value::as_str)
        .map(normalize_hash)
        .transpose()?;
    let mut build_digest = Sha256::new();
    build_digest.update(&build_bytes);
    let build_id = hex_bytes(&build_digest.finalize());
    Ok(ServiceArtifact {
        directory,
        blob_path,
        blob,
        code_hash,
        build_id,
        service_key,
        min_item_gas,
        min_memo_gas,
    })
}

fn verify_checksums(directory: &Path, path: &Path) -> Result<(), DeploymentError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Checksums {
        version: u8,
        algorithm: String,
        files: BTreeMap<String, String>,
    }
    let bytes = fs::read(path).map_err(|error| {
        DeploymentError::new(
            ErrorCode::ArtifactNotFound,
            format!("missing checksums.json: {error}"),
        )
    })?;
    let checksums: Checksums = serde_json::from_slice(&bytes).map_err(|error| {
        DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            format!("checksums.json is not valid: {error}"),
        )
    })?;
    if checksums.version != 1 || checksums.algorithm != "blake2b-256" || checksums.files.is_empty()
    {
        return Err(DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            "unsupported checksums.json format",
        ));
    }
    if !checksums.files.contains_key("service.blob") {
        return Err(DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            "checksums.json does not cover service.blob",
        ));
    }
    if !checksums.files.contains_key("build.json") {
        return Err(DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            "checksums.json does not cover build.json",
        ));
    }
    for (name, expected) in checksums.files {
        let relative = Path::new(&name);
        if name.is_empty()
            || relative.is_absolute()
            || name.contains('\\')
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(DeploymentError::new(
                ErrorCode::ArtifactInvalid,
                format!("unsafe checksum path '{name}'"),
            ));
        }
        let bytes = fs::read(directory.join(relative)).map_err(|error| {
            DeploymentError::new(
                ErrorCode::ArtifactInvalid,
                format!("checksum file '{name}' is missing: {error}"),
            )
        })?;
        let actual = blake2_hash(&bytes);
        let expected = normalize_hash(&expected)?;
        if actual != expected {
            return Err(DeploymentError::new(
                ErrorCode::ArtifactCodeHashMismatch,
                format!("checksum mismatch for artifact file '{name}'"),
            ));
        }
    }
    Ok(())
}

fn value_string(value: &serde_json::Value, name: &str) -> Result<String, DeploymentError> {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            DeploymentError::new(
                ErrorCode::ArtifactInvalid,
                format!("build.json is missing string field '{name}'"),
            )
        })
}

fn value_u64(value: &serde_json::Value, names: &[&str]) -> Result<u64, DeploymentError> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(serde_json::Value::as_u64))
        .ok_or_else(|| {
            DeploymentError::new(
                ErrorCode::ArtifactInvalid,
                format!("build.json is missing integer field '{}'", names[0]),
            )
        })
}

pub trait JsonRpcTransport {
    fn call(
        &self,
        endpoint: &str,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        mutating: bool,
    ) -> Result<serde_json::Value, DeploymentError>;
}

/// Register a successfully deployed Service with the application backend.
/// This is deliberately a separate control-plane call from on-chain Service
/// creation so a failed registration can be retried without creating another
/// Service.
pub fn register_backend_service<T: JsonRpcTransport>(
    transport: &T,
    endpoint: &str,
    admin_token: &str,
    service_id: u32,
    artifact: &ServiceArtifact,
    timeout: Duration,
) -> Result<serde_json::Value, DeploymentError> {
    let service_key = artifact.service_key.ok_or_else(|| {
        DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            "backend registration requires serviceKey in build.json",
        )
    })?;
    let planner_path = artifact.directory.join("generated_builder_application.rs");
    let planner_bytes = fs::read(&planner_path).map_err(|error| {
        DeploymentError::new(
            ErrorCode::BackendRegistrationFailed,
            format!(
                "cannot read backend planner artifact {}: {error}",
                planner_path.display()
            ),
        )
    })?;
    let planner_digest = blake2_hash(&planner_bytes);
    let result = transport
        .call(
            endpoint,
            "jamscript_registerServiceV1",
            serde_json::json!({
                "adminToken": admin_token,
                "serviceId": service_id,
                "serviceKey": hash_hex(&service_key),
                "codeHash": hash_hex(&artifact.code_hash),
                "abiVersion": 1,
                "plannerArtifact": {
                    "digest": hash_hex(&planner_digest),
                    "locator": planner_path.to_string_lossy(),
                },
            }),
            timeout,
            true,
        )
        .map_err(|error| {
            DeploymentError::new(
                ErrorCode::BackendRegistrationFailed,
                format!(
                    "on-chain Service {service_id} is deployed, but backend registration failed: {}",
                    error.message
                ),
            )
        })?;
    let returned_id = result
        .get("serviceId")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            DeploymentError::new(
                ErrorCode::BackendRegistrationFailed,
                "backend registration response omitted serviceId",
            )
        })?;
    if returned_id != service_id {
        return Err(DeploymentError::new(
            ErrorCode::BackendRegistrationFailed,
            format!("backend registered unexpected serviceId {returned_id}; expected {service_id}"),
        ));
    }
    Ok(result)
}

impl<T: JsonRpcTransport + ?Sized> JsonRpcTransport for &T {
    fn call(
        &self,
        endpoint: &str,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        mutating: bool,
    ) -> Result<serde_json::Value, DeploymentError> {
        (**self).call(endpoint, method, params, timeout, mutating)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CurlJsonRpcTransport;

impl JsonRpcTransport for CurlJsonRpcTransport {
    fn call(
        &self,
        endpoint: &str,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        mutating: bool,
    ) -> Result<serde_json::Value, DeploymentError> {
        let seconds = timeout.as_secs_f64().max(1.0).to_string();
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let mut child = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--location",
                "--connect-timeout",
                &seconds,
                "--max-time",
                &seconds,
                "--header",
                "content-type: application/json",
                "--data-binary",
                "@-",
                "--write-out",
                "\nJAMSCRIPT_HTTP_STATUS:%{http_code}",
                endpoint,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| {
                DeploymentError::new(
                    if mutating {
                        ErrorCode::DeploymentOutcomeUnknown
                    } else {
                        ErrorCode::NetworkUnreachable
                    },
                    format!("cannot run curl for {}: {error}", redact_url(endpoint)),
                )
            })?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(request.to_string().as_bytes())
                .map_err(|error| {
                    DeploymentError::new(
                        if mutating {
                            ErrorCode::DeploymentOutcomeUnknown
                        } else {
                            ErrorCode::NetworkUnreachable
                        },
                        format!(
                            "cannot send JSON-RPC request to {}: {error}",
                            redact_url(endpoint)
                        ),
                    )
                })?;
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().map_err(|error| {
            DeploymentError::new(
                if mutating {
                    ErrorCode::DeploymentOutcomeUnknown
                } else {
                    ErrorCode::NetworkUnreachable
                },
                format!("cannot finish curl for {}: {error}", redact_url(endpoint)),
            )
        })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let marker = "\nJAMSCRIPT_HTTP_STATUS:";
        let (body, status) = stdout.rsplit_once(marker).ok_or_else(|| {
            DeploymentError::new(
                if mutating {
                    ErrorCode::DeploymentOutcomeUnknown
                } else {
                    ErrorCode::NetworkUnreachable
                },
                format!(
                    "{} did not return an HTTP status from {}: {}",
                    method,
                    redact_url(endpoint),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
        })?;
        let status = status.trim().parse::<u16>().unwrap_or(0);
        if status == 0 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let connection_failure = stderr.contains("Could not resolve")
                || stderr.contains("Failed to connect")
                || stderr.contains("Connection refused")
                || stderr.contains("Could not connect");
            return Err(DeploymentError::new(
                if mutating && !connection_failure {
                    ErrorCode::DeploymentOutcomeUnknown
                } else {
                    ErrorCode::NetworkUnreachable
                },
                format!(
                    "{method}: {}",
                    stderr.trim().replace(endpoint, &redact_url(endpoint))
                ),
            ));
        }
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).map_err(|error| {
            DeploymentError::new(
                if mutating {
                    ErrorCode::DeploymentOutcomeUnknown
                } else {
                    ErrorCode::RpcInvalidResponse
                },
                format!(
                    "invalid JSON-RPC response from {}: {error}",
                    redact_url(endpoint)
                ),
            )
        })?;
        if status >= 400 {
            return Err(DeploymentError::new(
                if mutating {
                    ErrorCode::DeploymentRejected
                } else {
                    ErrorCode::NetworkUnreachable
                },
                format!("HTTP {status} from {}", redact_url(endpoint)),
            ));
        }
        parse_json_rpc_response(parsed, method, mutating)
    }
}

fn parse_json_rpc_response(
    value: serde_json::Value,
    method: &str,
    mutating: bool,
) -> Result<serde_json::Value, DeploymentError> {
    if value.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return Err(DeploymentError::new(
            if mutating {
                ErrorCode::DeploymentOutcomeUnknown
            } else {
                ErrorCode::RpcInvalidResponse
            },
            format!("{method} returned a non-JSON-RPC 2.0 response"),
        ));
    }
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(serde_json::Value::as_i64);
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("JSON-RPC error");
        let error_code = if code == Some(-32601) {
            ErrorCode::RpcMethodNotFound
        } else if mutating {
            ErrorCode::DeploymentRejected
        } else {
            ErrorCode::RpcInvalidResponse
        };
        return Err(DeploymentError::new(error_code, message.to_owned()));
    }
    value.get("result").cloned().ok_or_else(|| {
        DeploymentError::new(
            if mutating {
                ErrorCode::DeploymentOutcomeUnknown
            } else {
                ErrorCode::RpcInvalidResponse
            },
            format!("{method} response has neither result nor error"),
        )
    })
}

#[derive(Clone, Debug)]
pub struct CreateDeploymentRequest {
    pub artifact: ServiceArtifact,
}

#[derive(Clone, Debug, Serialize)]
pub struct NetworkIdentity {
    pub name: Option<String>,
    pub kind: NetworkKind,
    #[serde(rename = "genesisHash", skip_serializing_if = "Option::is_none")]
    pub genesis_hash: Option<String>,
    #[serde(skip)]
    pub verification: IdentityVerification,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IdentityVerification {
    Verified,
    Unpinned,
}

#[derive(Clone, Debug, Serialize)]
pub struct FinalizedContext {
    #[serde(rename = "blockHash")]
    pub block_hash: String,
    #[serde(rename = "blockNumber")]
    pub block_number: u64,
    #[serde(rename = "stateRoot", skip_serializing_if = "Option::is_none")]
    pub state_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeploymentResult {
    pub network: NetworkIdentity,
    #[serde(rename = "serviceId")]
    pub service_id: u32,
    #[serde(rename = "codeHash")]
    pub code_hash: String,
    pub finalized: bool,
    #[serde(rename = "finalizedBlock", skip_serializing_if = "Option::is_none")]
    pub finalized_context: Option<FinalizedContext>,
    #[serde(rename = "operationId", skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip)]
    pub artifact: ArtifactIdentity,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArtifactIdentity {
    #[serde(rename = "codeHash")]
    pub code_hash: String,
    #[serde(rename = "buildId")]
    pub build_id: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "serviceKey")]
    pub service_key: Option<String>,
}

pub trait DeploymentBackend {
    fn kind(&self) -> NetworkKind;
    fn create(
        &self,
        request: CreateDeploymentRequest,
        network: &ResolvedNetwork,
        timeout: Duration,
    ) -> Result<DeploymentResult, DeploymentError>;
}

pub struct MiniJamDeploymentBackend<T> {
    transport: T,
}

impl<T> MiniJamDeploymentBackend<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MiniJamCreateResponse {
    operation_id: String,
    service_id: u32,
    code_hash: String,
    finalized: bool,
    context: MiniJamContext,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct MiniJamContext {
    block_hash: String,
    block_number: u64,
    #[serde(default, rename = "stateRoot")]
    state_root: Option<String>,
    #[serde(default)]
    slot: Option<u64>,
}

impl<T: JsonRpcTransport> DeploymentBackend for MiniJamDeploymentBackend<T> {
    fn kind(&self) -> NetworkKind {
        NetworkKind::MiniJam
    }

    fn create(
        &self,
        request: CreateDeploymentRequest,
        network: &ResolvedNetwork,
        timeout: Duration,
    ) -> Result<DeploymentResult, DeploymentError> {
        let artifact = request.artifact;
        let value = self.transport.call(
            &network.deployment_rpc,
            "minijam_createServiceV1",
            serde_json::json!({
                "codeHash": hash_hex(&artifact.code_hash),
                "blobBase64": BASE64.encode(&artifact.blob),
                "minItemGas": artifact.min_item_gas,
                "minMemoGas": artifact.min_memo_gas,
            }),
            timeout,
            true,
        )?;
        let response: MiniJamCreateResponse = serde_json::from_value(value).map_err(|error| {
            DeploymentError::new(
                ErrorCode::RpcInvalidResponse,
                format!("invalid minijam_createServiceV1 result: {error}"),
            )
        })?;
        let returned_hash = normalize_rpc_hash(&response.code_hash, "codeHash")?;
        let block_hash = normalize_rpc_hash(&response.context.block_hash, "context.blockHash")
            .map(|hash| hash_hex(&hash))?;
        let state_root = response
            .context
            .state_root
            .as_deref()
            .map(|value| normalize_rpc_hash(value, "context.stateRoot"))
            .transpose()?
            .map(|value| hash_hex(&value));
        if !response.finalized {
            return Err(DeploymentError::new(
                ErrorCode::DeploymentNotFinalized,
                "MiniJAM create-Service response was not finalized",
            ));
        }
        if returned_hash != artifact.code_hash {
            return Err(DeploymentError::new(
                ErrorCode::DeploymentCodeHashMismatch,
                format!(
                    "MiniJAM returned codeHash {} but local artifact is {}",
                    hash_hex(&returned_hash),
                    hash_hex(&artifact.code_hash)
                ),
            ));
        }
        if response.operation_id.trim().is_empty() {
            return Err(DeploymentError::new(
                ErrorCode::RpcInvalidResponse,
                "MiniJAM deployment response has an empty operationId",
            ));
        }
        Ok(DeploymentResult {
            network: NetworkIdentity {
                name: network.name.clone(),
                kind: network.kind.clone(),
                genesis_hash: network.genesis_hash.clone(),
                verification: if network.genesis_pinned {
                    IdentityVerification::Verified
                } else {
                    IdentityVerification::Unpinned
                },
            },
            service_id: response.service_id,
            code_hash: hash_hex(&returned_hash),
            finalized: true,
            finalized_context: Some(FinalizedContext {
                block_hash,
                block_number: response.context.block_number,
                state_root,
                slot: response.context.slot,
            }),
            operation_id: Some(response.operation_id),
            artifact: ArtifactIdentity {
                code_hash: hash_hex(&artifact.code_hash),
                build_id: artifact.build_id,
                service_key: artifact.service_key.map(|value| hash_hex(&value)),
            },
        })
    }
}

pub struct DeploymentEngine<T> {
    transport: T,
}

impl<T> DeploymentEngine<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: JsonRpcTransport> DeploymentEngine<T> {
    pub fn deploy(
        &self,
        network: ResolvedNetwork,
        artifact: ServiceArtifact,
        timeout: Duration,
    ) -> Result<DeploymentResult, DeploymentError> {
        if network.kind != NetworkKind::MiniJam {
            return Err(DeploymentError::new(
                ErrorCode::UnsupportedNetworkKind,
                "JAM deployment is not supported in v0.1",
            ));
        }
        let observed_genesis = if let Some(node_rpc) = network.node_rpc.as_deref() {
            Some(self.read_genesis(node_rpc, timeout)?)
        } else {
            None
        };
        if let Some(expected) = network.genesis_hash.as_deref() {
            if Some(expected) != observed_genesis.as_deref() {
                return Err(DeploymentError::new(
                    ErrorCode::NetworkIdentityMismatch,
                    format!(
                        "NETWORK_IDENTITY_MISMATCH\n\nConfigured genesis:\n  {expected}\n\nConnected genesis:\n  {}\n\nNo deployment was submitted.",
                        observed_genesis.as_deref().unwrap_or("<unavailable>")
                    ),
                ));
            }
        }
        let mut network = network;
        network.genesis_hash = observed_genesis.or(network.genesis_hash);
        MiniJamDeploymentBackend::new(&self.transport).create(
            CreateDeploymentRequest { artifact },
            &network,
            timeout,
        )
    }

    fn read_genesis(&self, endpoint: &str, timeout: Duration) -> Result<String, DeploymentError> {
        let value = self.transport.call(
            endpoint,
            "chain_getBlockHash",
            serde_json::json!([0]),
            timeout,
            false,
        )?;
        let value = value.as_str().ok_or_else(|| {
            DeploymentError::new(
                ErrorCode::RpcInvalidResponse,
                "chain_getBlockHash returned a non-string result",
            )
        })?;
        normalize_rpc_hash(value, "chain_getBlockHash").map(|hash| hash_hex(&hash))
    }
}

fn normalize_rpc_hash(value: &str, field: &str) -> Result<[u8; 32], DeploymentError> {
    normalize_hash(value).map_err(|error| {
        DeploymentError::new(
            ErrorCode::RpcInvalidResponse,
            format!(
                "{field} is not a 32-byte hexadecimal hash: {}",
                error.message
            ),
        )
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct DeploymentRecord {
    pub format: u8,
    pub network: NetworkIdentity,
    pub artifact: ArtifactIdentity,
    pub service: ServiceIdentity,
    pub finalized: FinalizedContext,
    pub backend: BackendIdentity,
}

#[derive(Clone, Debug, Serialize)]
pub struct ServiceIdentity {
    pub id: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct BackendIdentity {
    pub protocol: &'static str,
    #[serde(rename = "operationId", skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

pub fn write_deployment_record(
    project_root: &Path,
    result: &DeploymentResult,
) -> Result<PathBuf, DeploymentError> {
    let finalized = result.finalized_context.clone().ok_or_else(|| {
        DeploymentError::new(
            ErrorCode::DeploymentNotFinalized,
            "cannot record a deployment without finalized context",
        )
    })?;
    let record = DeploymentRecord {
        format: 1,
        network: result.network.clone(),
        artifact: result.artifact.clone(),
        service: ServiceIdentity {
            id: result.service_id,
        },
        finalized,
        backend: BackendIdentity {
            protocol: "minijam-create-service-v1",
            operation_id: result.operation_id.clone(),
        },
    };
    let directory = project_root.join(".jamscript/deployments");
    fs::create_dir_all(&directory).map_err(|error| {
        DeploymentError::new(
            ErrorCode::DeploymentRecordWriteFailed,
            format!("cannot create deployment record directory: {error}"),
        )
    })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!(
        "{}-{}-{timestamp}.json",
        safe_file_component(result.network.name.as_deref().unwrap_or("custom")),
        result.service_id
    ));
    let temporary = directory.join(format!(".{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&record).map_err(|error| {
        DeploymentError::new(
            ErrorCode::DeploymentRecordWriteFailed,
            format!("cannot encode deployment record: {error}"),
        )
    })?;
    fs::write(&temporary, bytes)
        .and_then(|_| fs::rename(&temporary, &path))
        .map_err(|error| {
            DeploymentError::new(
                ErrorCode::DeploymentRecordWriteFailed,
                format!("cannot write deployment record {}: {error}", path.display()),
            )
        })?;
    Ok(path)
}

fn safe_file_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "network".to_owned()
    } else {
        sanitized
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ErrorCode {
    #[error("NETWORK_NOT_FOUND")]
    NetworkNotFound,
    #[error("NETWORK_CONFIG_INVALID")]
    NetworkConfigInvalid,
    #[error("UNSUPPORTED_NETWORK_KIND")]
    UnsupportedNetworkKind,
    #[error("NETWORK_UNREACHABLE")]
    NetworkUnreachable,
    #[error("NETWORK_IDENTITY_MISMATCH")]
    NetworkIdentityMismatch,
    #[error("ARTIFACT_NOT_FOUND")]
    ArtifactNotFound,
    #[error("ARTIFACT_INVALID")]
    ArtifactInvalid,
    #[error("ARTIFACT_CODE_HASH_MISMATCH")]
    ArtifactCodeHashMismatch,
    #[error("RPC_INVALID_RESPONSE")]
    RpcInvalidResponse,
    #[error("RPC_METHOD_NOT_FOUND")]
    RpcMethodNotFound,
    #[error("DEPLOYMENT_REJECTED")]
    DeploymentRejected,
    #[error("DEPLOYMENT_NOT_FINALIZED")]
    DeploymentNotFinalized,
    #[error("DEPLOYMENT_CODE_HASH_MISMATCH")]
    DeploymentCodeHashMismatch,
    #[error("DEPLOYMENT_OUTCOME_UNKNOWN")]
    DeploymentOutcomeUnknown,
    #[error("BACKEND_REGISTRATION_FAILED")]
    BackendRegistrationFailed,
    #[error("DEPLOYMENT_RECORD_WRITE_FAILED")]
    DeploymentRecordWriteFailed,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{code}: {message}")]
pub struct DeploymentError {
    pub code: ErrorCode,
    pub message: String,
}

impl DeploymentError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn normalize_hash(value: &str) -> Result<[u8; 32], DeploymentError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() != 64 {
        return Err(DeploymentError::new(
            ErrorCode::ArtifactInvalid,
            "hash must contain exactly 32 bytes of hexadecimal data",
        ));
    }
    let mut output = [0u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
            DeploymentError::new(
                ErrorCode::ArtifactInvalid,
                "hash contains invalid hexadecimal data",
            )
        })?;
    }
    Ok(output)
}

fn blake2_hash(bytes: &[u8]) -> [u8; 32] {
    Blake2Params::new()
        .hash_length(32)
        .hash(bytes)
        .as_bytes()
        .try_into()
        .expect("32-byte Blake2 hash")
}

pub fn hash_hex(value: &[u8; 32]) -> String {
    format!("0x{}", hex_bytes(value))
}

fn hex_bytes(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MockTransport {
        calls: Arc<Mutex<Vec<(String, serde_json::Value, bool)>>>,
        responses: Arc<Mutex<Vec<Result<serde_json::Value, DeploymentError>>>>,
    }

    impl MockTransport {
        fn with_responses(responses: Vec<Result<serde_json::Value, DeploymentError>>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    impl JsonRpcTransport for MockTransport {
        fn call(
            &self,
            endpoint: &str,
            method: &str,
            params: serde_json::Value,
            _timeout: Duration,
            mutating: bool,
        ) -> Result<serde_json::Value, DeploymentError> {
            self.calls
                .lock()
                .unwrap()
                .push((format!("{endpoint}:{method}"), params, mutating));
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn network() -> ResolvedNetwork {
        ResolvedNetwork {
            name: Some("local".into()),
            kind: NetworkKind::MiniJam,
            deployment_rpc: "http://deploy.test".into(),
            node_rpc: None,
            backend_rpc: None,
            genesis_hash: None,
            genesis_pinned: false,
        }
    }

    fn artifact() -> ServiceArtifact {
        let blob = b"canonical-service".to_vec();
        ServiceArtifact {
            directory: PathBuf::from("dist"),
            blob_path: PathBuf::from("dist/service.blob"),
            code_hash: blake2_hash(&blob),
            blob,
            build_id: "build-id".into(),
            service_key: None,
            min_item_gas: 1,
            min_memo_gas: 1,
        }
    }

    fn response(artifact: &ServiceArtifact) -> serde_json::Value {
        serde_json::json!({
            "operationId": "0xop",
            "serviceId": 7,
            "codeHash": hash_hex(&artifact.code_hash),
            "finalized": true,
            "context": { "blockHash": format!("0x{}", "11".repeat(32)), "blockNumber": 12 }
        })
    }

    #[test]
    fn resolves_named_networks_and_explicit_default() {
        let table = BTreeMap::from([
            (
                "local".into(),
                NetworkConfig {
                    kind: "minijam".into(),
                    deployment_rpc: Some("http://127.0.0.1:8090".into()),
                    node_rpc: None,
                    backend_rpc: None,
                    genesis_hash: None,
                },
            ),
            (
                "other".into(),
                NetworkConfig {
                    kind: "minijam".into(),
                    deployment_rpc: Some("https://community.example".into()),
                    node_rpc: None,
                    backend_rpc: None,
                    genesis_hash: None,
                },
            ),
        ]);
        let resolved = resolve_network(
            Some(&table),
            Some(&DeploymentConfig {
                default_network: Some("other".into()),
            }),
            NetworkOverrides::default(),
        )
        .unwrap();
        assert_eq!(resolved.display_name(), "other");
        assert_eq!(resolved.deployment_rpc, "https://community.example");
        let selected = resolve_network(
            Some(&table),
            None,
            NetworkOverrides {
                network: Some("local".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(selected.deployment_rpc, "http://127.0.0.1:8090");
    }

    #[test]
    fn explicit_network_settings_override_the_selected_network() {
        let table = BTreeMap::from([
            (
                "local".into(),
                NetworkConfig {
                    kind: "minijam".into(),
                    deployment_rpc: Some("http://local.example".into()),
                    node_rpc: None,
                    backend_rpc: None,
                    genesis_hash: None,
                },
            ),
            (
                "staging".into(),
                NetworkConfig {
                    kind: "minijam".into(),
                    deployment_rpc: Some("https://staging.example".into()),
                    node_rpc: Some("https://staging-node.example".into()),
                    backend_rpc: Some("https://staging-backend.example".into()),
                    genesis_hash: None,
                },
            ),
        ]);
        let resolved = resolve_network(
            Some(&table),
            None,
            NetworkOverrides {
                network: Some("staging".into()),
                deployment_rpc: Some("http://operator.example".into()),
                node_rpc: Some("http://operator-node.example".into()),
                backend_rpc: Some("http://operator-backend.example".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.display_name(), "staging");
        assert_eq!(resolved.deployment_rpc, "http://operator.example");
        assert_eq!(
            resolved.node_rpc.as_deref(),
            Some("http://operator-node.example")
        );
        assert_eq!(
            resolved.backend_rpc.as_deref(),
            Some("http://operator-backend.example")
        );
    }

    #[test]
    fn custom_minijam_mode_does_not_require_a_named_network() {
        let resolved = resolve_network(
            None,
            None,
            NetworkOverrides {
                kind: Some("minijam".into()),
                deployment_rpc: Some("https://deployment.example".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.display_name(), "custom");
        assert_eq!(resolved.kind, NetworkKind::MiniJam);
    }

    #[test]
    fn deploy_uses_exact_stage1_method_and_does_not_retry_mutation() {
        let artifact = artifact();
        let transport = MockTransport::with_responses(vec![Ok(response(&artifact))]);
        let calls = transport.calls.clone();
        let result = DeploymentEngine::new(transport)
            .deploy(network(), artifact, Duration::from_secs(2))
            .unwrap();
        assert_eq!(result.service_id, 7);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.ends_with(":minijam_createServiceV1"));
        assert!(calls[0].2);
    }

    #[test]
    fn backend_registration_is_separate_and_contains_planner_identity() {
        let directory = tempfile::tempdir().unwrap();
        let planner_path = directory.path().join("generated_builder_application.rs");
        fs::write(&planner_path, b"portable planner").unwrap();
        let mut artifact = artifact();
        artifact.directory = directory.path().to_path_buf();
        artifact.service_key = Some([7; 32]);
        let transport = MockTransport::with_responses(vec![Ok(serde_json::json!({
            "serviceId": 7,
        }))]);
        let result = register_backend_service(
            &transport,
            "http://backend.test",
            "admin",
            7,
            &artifact,
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(result["serviceId"], 7);
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "http://backend.test:jamscript_registerServiceV1"
        );
        assert_eq!(calls[0].1["serviceKey"], hash_hex(&[7; 32]));
        assert!(calls[0].1["plannerArtifact"]["digest"].as_str().is_some());
    }

    #[test]
    fn genesis_mismatch_happens_before_create() {
        let transport = MockTransport::with_responses(vec![Ok(serde_json::json!(format!(
            "0x{}",
            "bb".repeat(32)
        )))]);
        let calls = transport.calls.clone();
        let mut configured = network();
        configured.node_rpc = Some("http://node.test".into());
        configured.genesis_hash = Some(format!("0x{}", "aa".repeat(32)));
        let result =
            DeploymentEngine::new(transport).deploy(configured, artifact(), Duration::from_secs(2));
        assert_eq!(result.unwrap_err().code, ErrorCode::NetworkIdentityMismatch);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(!calls.lock().unwrap()[0].2);
    }

    #[test]
    fn jam_is_recognized_but_not_implemented() {
        let table = BTreeMap::from([(
            "mainnet".into(),
            NetworkConfig {
                kind: "jam".into(),
                deployment_rpc: None,
                node_rpc: None,
                backend_rpc: None,
                genesis_hash: None,
            },
        )]);
        let error = resolve_network(
            Some(&table),
            None,
            NetworkOverrides {
                network: Some("mainnet".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedNetworkKind);
    }

    #[test]
    fn secret_url_is_redacted() {
        assert_eq!(
            redact_url("https://user:secret@example.org/?token=abc#fragment"),
            "https://example.org/"
        );
    }

    fn write_artifact_fixture(directory: &Path) {
        let blob = b"canonical-service";
        fs::write(directory.join("service.blob"), blob).unwrap();
        let metadata = serde_json::json!({
            "code_hash": hash_hex(&blake2_hash(blob)),
            "minItemGas": 5_000_000,
            "minMemoGas": 1_000_000,
            "serviceKey": format!("0x{}", "22".repeat(32)),
        });
        let build_bytes = serde_json::to_vec_pretty(&metadata).unwrap();
        fs::write(directory.join("build.json"), &build_bytes).unwrap();
        let files = BTreeMap::from([
            ("service.blob".to_owned(), hash_hex(&blake2_hash(blob))),
            (
                "build.json".to_owned(),
                hash_hex(&blake2_hash(&build_bytes)),
            ),
        ]);
        fs::write(
            directory.join("checksums.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1,
                "algorithm": "blake2b-256",
                "files": files,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn artifact_preflight_requires_integrity_for_build_metadata() {
        let temp = tempfile::tempdir().unwrap();
        write_artifact_fixture(temp.path());
        let artifact = load_service_artifact(temp.path()).unwrap();
        assert_eq!(artifact.min_item_gas, 5_000_000);
        assert_eq!(artifact.min_memo_gas, 1_000_000);
        assert_eq!(artifact.service_key, Some([0x22; 32]));

        fs::write(
            temp.path().join("build.json"),
            serde_json::json!({
                "code_hash": hash_hex(&artifact.code_hash),
                "minItemGas": 1,
                "minMemoGas": 1,
            })
            .to_string(),
        )
        .unwrap();
        let error = load_service_artifact(temp.path()).unwrap_err();
        assert_eq!(error.code, ErrorCode::ArtifactCodeHashMismatch);
    }
}
