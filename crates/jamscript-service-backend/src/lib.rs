//! Multi-Service backend state and identity boundaries.
//!
//! The backend owns routing and materialized state, while JAM remains the
//! authority for canonical commitments.  This crate intentionally does not
//! embed a generated application: artifacts are registered per Service and a
//! daemon can route many Services without being rebuilt.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use service_runtime_core::{
    ExecutionContext, RuntimeRefineOutputV1, ServiceApplication, ServiceKeyV1, StateAccessError,
    StateQueryResponseV1, StateRoot, WireError, MAX_RECOVERY_BYTES, RECOVERY_FORMAT_VERSION,
};
use service_runtime_host::{
    FullStateProvider, MaterializedServiceStateProvider, ProviderError, ServiceStateProvider,
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::Arc,
    time::Duration,
};

pub const BACKEND_PROTOCOL_VERSION_V1: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationArtifactRef {
    pub digest: StateRoot,
    pub locator: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct DeploymentMetadata {
    pub manifest_digest: Option<StateRoot>,
    pub registered_at: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceRecord {
    pub service_id: u32,
    pub service_key: ServiceKeyV1,
    pub code_hash: StateRoot,
    pub abi_version: u32,
    pub application_artifact: ApplicationArtifactRef,
    pub deployment: DeploymentMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    ServiceIdAlreadyBound,
    ServiceKeyAlreadyBound,
    ServiceIdKeyMismatch,
    UnknownServiceId,
    UnknownServiceKey,
}

#[derive(Clone, Debug, Default)]
pub struct ServiceRegistry {
    by_id: BTreeMap<u32, ServiceRecord>,
    by_key: BTreeMap<ServiceKeyV1, u32>,
}

impl ServiceRegistry {
    pub fn register(&mut self, record: ServiceRecord) -> Result<(), RegistryError> {
        if let Some(existing) = self.by_id.get(&record.service_id) {
            if existing.service_key != record.service_key {
                return Err(RegistryError::ServiceIdAlreadyBound);
            }
            // Code/artifact upgrades preserve the chain identity. The key is
            // the immutable binding that prevents accidental cross-routing.
            self.by_id.insert(record.service_id, record);
            return Ok(());
        }
        if let Some(existing_id) = self.by_key.get(&record.service_key) {
            if *existing_id != record.service_id {
                return Err(RegistryError::ServiceKeyAlreadyBound);
            }
            return Err(RegistryError::ServiceIdKeyMismatch);
        }
        self.by_key.insert(record.service_key, record.service_id);
        self.by_id.insert(record.service_id, record);
        Ok(())
    }

    pub fn get(&self, service_id: u32) -> Result<&ServiceRecord, RegistryError> {
        self.by_id
            .get(&service_id)
            .ok_or(RegistryError::UnknownServiceId)
    }

    pub fn get_by_key(&self, service_key: ServiceKeyV1) -> Result<&ServiceRecord, RegistryError> {
        let service_id = self
            .by_key
            .get(&service_key)
            .ok_or(RegistryError::UnknownServiceKey)?;
        self.get(*service_id)
    }

    pub fn resolve_key(&self, service_id: u32) -> Result<ServiceKeyV1, RegistryError> {
        Ok(self.get(service_id)?.service_key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ServiceRecord> {
        self.by_id.values()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct WorkKey {
    pub service_id: u32,
    pub package_hash: StateRoot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannerRequest {
    pub actions: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct PlannerResult {
    pub local_access_keys: Vec<Vec<u8>>,
    pub external_access_keys: BTreeMap<u32, Vec<Vec<u8>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlannerError {
    InvalidArtifact,
    ApplicationRejected,
    Backend,
}

/// Portable planner boundary. A deployed artifact is bound to a Service at
/// registration time and can be swapped/upgraded without recompiling this
/// daemon. The implementation may be a WASM/PVM/ScriptC artifact loader.
pub trait ApplicationPlanner: Send + Sync {
    fn artifact_digest(&self) -> StateRoot;
    fn plan(&self, request: &PlannerRequest) -> Result<PlannerResult, PlannerError>;

    /// Optional execution half of a portable application artifact. Planners
    /// that only support access discovery can keep the default and will be
    /// rejected when a work builder tries to execute them.
    fn execute(
        &self,
        _context: &mut ExecutionContext<'_>,
        _input: &[u8],
    ) -> Result<(), StateAccessError> {
        Err(StateAccessError::Backend)
    }
}

/// A loader turns a registered portable artifact into a planner/runtime
/// binding. Implementations can use a WASM/PVM/ScriptC artifact without
/// compiling the backend daemon again.
pub trait ApplicationArtifactLoader: Send + Sync {
    fn load(
        &self,
        artifact: &ApplicationArtifactRef,
    ) -> Result<Arc<dyn ApplicationPlanner>, PlannerError>;
}

pub struct RegisteredApplication<'a> {
    application: &'a dyn ApplicationPlanner,
}

impl<'a> RegisteredApplication<'a> {
    pub fn new(application: &'a dyn ApplicationPlanner) -> Self {
        Self { application }
    }
}

impl ServiceApplication for RegisteredApplication<'_> {
    type Error = StateAccessError;

    fn execute(&self, context: &mut ExecutionContext<'_>, input: &[u8]) -> Result<(), Self::Error> {
        self.application.execute(context, input)
    }
}

#[derive(Clone, Default)]
pub struct ApplicationRegistry {
    by_service_id: BTreeMap<u32, Arc<dyn ApplicationPlanner>>,
}

impl ApplicationRegistry {
    pub fn bind(
        &mut self,
        service: &ServiceRecord,
        planner: Arc<dyn ApplicationPlanner>,
    ) -> Result<(), BackendError> {
        if planner.artifact_digest() != service.application_artifact.digest {
            return Err(BackendError::PlannerArtifactMismatch);
        }
        self.by_service_id.insert(service.service_id, planner);
        Ok(())
    }

    pub fn get(&self, service_id: u32) -> Result<&Arc<dyn ApplicationPlanner>, BackendError> {
        self.by_service_id
            .get(&service_id)
            .ok_or(BackendError::PlannerUnavailable)
    }

    pub fn remove(&mut self, service_id: u32) {
        self.by_service_id.remove(&service_id);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryEnvelopeV1 {
    pub service_id: u32,
    pub service_key: ServiceKeyV1,
    pub output: RuntimeRefineOutputV1,
}

impl RecoveryEnvelopeV1 {
    pub const VERSION: u8 = RECOVERY_FORMAT_VERSION;

    pub fn encode(&self) -> Result<Vec<u8>, BackendError> {
        let output = self.output.encode().map_err(BackendError::Wire)?;
        let length = u32::try_from(output.len()).map_err(|_| BackendError::EntryTooLarge)?;
        let mut encoded = Vec::with_capacity(1 + 4 + 32 + 4 + output.len());
        encoded.push(Self::VERSION);
        encoded.extend_from_slice(&self.service_id.to_le_bytes());
        encoded.extend_from_slice(self.service_key.as_bytes());
        encoded.extend_from_slice(&length.to_le_bytes());
        encoded.extend_from_slice(&output);
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, BackendError> {
        if bytes.len() > MAX_RECOVERY_BYTES + 128 * 1024 {
            return Err(BackendError::EntryTooLarge);
        }
        let mut reader = EnvelopeReader { bytes, offset: 0 };
        if reader.u8()? != Self::VERSION {
            return Err(BackendError::InvalidEnvelope);
        }
        let service_id = reader.u32()?;
        let service_key = ServiceKeyV1::new(reader.array::<32>()?);
        let output = RuntimeRefineOutputV1::decode(&reader.bytes(MAX_RECOVERY_BYTES + 128 * 1024)?)
            .map_err(BackendError::Wire)?;
        if reader.remaining() != 0 {
            return Err(BackendError::InvalidEnvelope);
        }
        Ok(Self {
            service_id,
            service_key,
            output,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    Registry(RegistryError),
    Provider(ProviderError),
    Wire(WireError),
    InvalidEnvelope,
    EntryTooLarge,
    TruncatedLog,
    UnknownService,
    ServiceKeyMismatch,
    RecoveryNotCanonical,
    PlannerArtifactMismatch,
    PlannerUnavailable,
    Planner(PlannerError),
    Rpc(String),
}

impl From<RegistryError> for BackendError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error)
    }
}

impl From<ProviderError> for BackendError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CapabilitiesV1 {
    pub protocol_version: u32,
    pub managed_state_version: u32,
    pub multi_service: bool,
    pub external_state_witness: bool,
}

impl Default for CapabilitiesV1 {
    fn default() -> Self {
        Self {
            protocol_version: BACKEND_PROTOCOL_VERSION_V1,
            managed_state_version: 1,
            multi_service: true,
            external_state_witness: true,
        }
    }
}

/// A chain-facing registration validator. The public backend must not accept
/// an arbitrary `(service_id, code_hash)` claim without checking it against
/// canonical Service metadata first.
pub trait ServiceRegistrationValidator: Send + Sync {
    fn validate(&self, record: &ServiceRecord) -> Result<(), BackendError>;
}

/// The network-specific portion of the backend. The shared daemon owns
/// Service routing, proof-backed state, and recovery; this narrow trait keeps
/// node/Formal RPC details below the application-facing endpoint.
pub trait BackendWorkGateway: Send + Sync {
    fn submit_work(&self, params: Value) -> Result<Value, BackendError>;
    fn work_status(&self, params: Value) -> Result<Value, BackendError>;
}

#[derive(Default)]
pub struct UnconfiguredWorkGateway;

impl BackendWorkGateway for UnconfiguredWorkGateway {
    fn submit_work(&self, _params: Value) -> Result<Value, BackendError> {
        Err(BackendError::Rpc("work gateway is not configured".into()))
    }

    fn work_status(&self, _params: Value) -> Result<Value, BackendError> {
        Err(BackendError::Rpc("work gateway is not configured".into()))
    }
}

/// One application-facing JSON-RPC surface for a multi-Service backend.
/// Network adapters supply [`BackendWorkGateway`] while this handler keeps
/// node/Formal topology and Service identity out of frontend configuration.
pub struct BackendRpcHandler {
    pub state: std::sync::Mutex<BackendState>,
    work: Arc<dyn BackendWorkGateway>,
    registration_validator: Option<Arc<dyn ServiceRegistrationValidator>>,
    artifact_loader: Option<Arc<dyn ApplicationArtifactLoader>>,
    admin_token: Option<String>,
}

impl BackendRpcHandler {
    pub fn new(state: BackendState, work: Arc<dyn BackendWorkGateway>) -> Self {
        Self {
            state: std::sync::Mutex::new(state),
            work,
            registration_validator: None,
            artifact_loader: None,
            admin_token: None,
        }
    }

    pub fn with_registration_validator(
        mut self,
        validator: Arc<dyn ServiceRegistrationValidator>,
    ) -> Self {
        self.registration_validator = Some(validator);
        self
    }

    pub fn with_artifact_loader(mut self, loader: Arc<dyn ApplicationArtifactLoader>) -> Self {
        self.artifact_loader = Some(loader);
        self
    }

    /// Require an internal control-plane token for Service registration. A
    /// handler without a token deliberately rejects registration rather than
    /// exposing an unrestricted public write API.
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin_token = Some(token.into());
        self
    }

    pub fn handle(&self, method: &str, params: Value) -> Result<Value, BackendError> {
        match method {
            "jamscript_getCapabilitiesV1" => Ok(serde_json::to_value(CapabilitiesJson::default())
                .map_err(|error| BackendError::Rpc(error.to_string()))?),
            "jamscript_listServicesV1" => self.list_services(),
            "jamscript_registerServiceV1" => self.register_service(params),
            "minijam_submitWorkV1" => self.submit_work(params),
            "minijam_getWorkStatusV1" => self.work_status(params),
            "minijam_getManagedStateV1" => self.get_managed_state(params),
            "jamscript_getPredictionV1" => self.get_prediction(params),
            _ => Err(BackendError::Rpc(format!("method not found: {method}"))),
        }
    }

    /// Handle a complete JSON-RPC 2.0 body. HTTP servers can use this method
    /// directly, and tests can exercise the same public contract in-process.
    pub fn handle_json(&self, body: &[u8]) -> Vec<u8> {
        let response = match serde_json::from_slice::<RpcRequest>(body) {
            Ok(request) => match self.handle(&request.method, request.params) {
                Ok(result) => json!({"jsonrpc":"2.0","id":request.id,"result":result}),
                Err(error) => json!({
                    "jsonrpc":"2.0",
                    "id":request.id,
                    "error": rpc_error(&error),
                }),
            },
            Err(error) => json!({
                "jsonrpc":"2.0",
                "id":Value::Null,
                "error":{"code":-32600,"message":error.to_string()},
            }),
        };
        response.to_string().into_bytes()
    }

    fn list_services(&self) -> Result<Value, BackendError> {
        let state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        state
            .registry
            .iter()
            .map(service_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    }

    fn register_service(&self, params: Value) -> Result<Value, BackendError> {
        let object = params
            .as_object()
            .ok_or_else(|| BackendError::Rpc("registration params must be an object".into()))?;
        let supplied_token = object.get("adminToken").and_then(Value::as_str);
        if self
            .admin_token
            .as_deref()
            .is_none_or(|token| supplied_token != Some(token))
        {
            return Err(BackendError::Rpc(
                "Service registration requires the backend control-plane token".into(),
            ));
        }
        let record = parse_service_record(&params)?;
        if let Some(validator) = &self.registration_validator {
            validator.validate(&record)?;
        }
        let planner = self
            .artifact_loader
            .as_ref()
            .map(|loader| {
                loader
                    .load(&record.application_artifact)
                    .map_err(BackendError::Planner)
            })
            .transpose()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        if let Ok(existing) = state.registry.get(record.service_id) {
            let upgrade = object
                .get("allowUpgrade")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if existing.service_key != record.service_key || !upgrade {
                return Err(BackendError::Rpc(
                    "Service is already registered; use an explicit validated upgrade".into(),
                ));
            }
        }
        state.register(record.clone())?;
        if let Some(planner) = planner {
            state.register_planner(record.service_id, planner)?;
        }
        service_json(&record)
    }

    fn get_managed_state(&self, params: Value) -> Result<Value, BackendError> {
        let service_id = required_u32(&params, "serviceId")?;
        let root = parse_hash(required_str(&params, "stateRoot")?)?;
        let key = BASE64
            .decode(required_str(&params, "keyBase64")?)
            .map_err(|error| BackendError::Rpc(format!("invalid keyBase64: {error}")))?;
        let state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        let record = state.registry.get(service_id)?;
        let response = state
            .provider
            .get(record.service_key, root, &key)
            .map_err(BackendError::Provider)?;
        query_json(service_id, &response)
    }

    fn submit_work(&self, params: Value) -> Result<Value, BackendError> {
        let service_id = required_u32(&params, "serviceId")?;
        let service_code_hash = parse_hash(required_str(&params, "serviceCodeHash")?)?;
        let state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        let record = state.registry.get(service_id)?;
        if record.code_hash != service_code_hash {
            return Err(BackendError::Rpc("Service code hash mismatch".into()));
        }
        drop(state);
        self.work.submit_work(params)
    }

    fn work_status(&self, params: Value) -> Result<Value, BackendError> {
        if let Some(service_id) = params.get("serviceId") {
            let service_id = service_id
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| BackendError::Rpc("serviceId must be a u32".into()))?;
            self.state
                .lock()
                .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?
                .registry
                .get(service_id)?;
        }
        self.work.work_status(params)
    }

    fn get_prediction(&self, params: Value) -> Result<Value, BackendError> {
        let service_id = required_u32(&params, "serviceId")?;
        let package_hash = parse_hash(required_str(&params, "packageHash")?)?;
        let state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        let output = state
            .prediction(service_id, package_hash)
            .ok_or_else(|| BackendError::Rpc("prediction not found".into()))?;
        Ok(json!({
            "serviceId": service_id,
            "packageHash": hash_hex(&package_hash),
            "parentRoot": hash_hex(&output.parent_root),
            "newRoot": hash_hex(&output.new_root),
            "externalDependencies": output.external_dependencies.iter().map(|dependency| json!({
                "serviceId": dependency.service_id,
                "stateRoot": hash_hex(&dependency.state_root),
            })).collect::<Vec<_>>(),
            "validUntil": output.transition_valid_until,
        }))
    }
}

#[derive(serde::Deserialize)]
struct RpcRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesJson {
    protocol_version: u32,
    managed_state_version: u32,
    multi_service: bool,
    external_state_witness: bool,
}

impl Default for CapabilitiesJson {
    fn default() -> Self {
        let capabilities = CapabilitiesV1::default();
        Self {
            protocol_version: capabilities.protocol_version,
            managed_state_version: capabilities.managed_state_version,
            multi_service: capabilities.multi_service,
            external_state_witness: capabilities.external_state_witness,
        }
    }
}

/// Minimal single-endpoint HTTP daemon. A production deployment can put TLS
/// and authentication in front of this listener; the routing/state contract
/// remains the same.
pub struct BackendDaemon {
    bind: String,
    handler: Arc<BackendRpcHandler>,
}

impl BackendDaemon {
    pub fn new(bind: impl Into<String>, handler: Arc<BackendRpcHandler>) -> Self {
        Self {
            bind: bind.into(),
            handler,
        }
    }

    pub fn bind(&self) -> &str {
        &self.bind
    }

    pub fn serve(&self) -> Result<(), BackendError> {
        let listener =
            TcpListener::bind(&self.bind).map_err(|error| BackendError::Rpc(error.to_string()))?;
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let handler = Arc::clone(&self.handler);
                    std::thread::spawn(move || {
                        let _ = serve_connection(stream, &handler);
                    });
                }
                Err(error) => return Err(BackendError::Rpc(error.to_string())),
            }
        }
        Ok(())
    }
}

fn serve_connection(
    mut stream: TcpStream,
    handler: &BackendRpcHandler,
) -> Result<(), BackendError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| BackendError::Rpc(error.to_string()))?;
    let (method, path, body) = read_http_request(&mut stream)?;
    let (status, response) = if method == "GET" && path == "/health/ready" {
        ("200 OK", br#"{"status":"ready"}"#.to_vec())
    } else if method == "POST" && path == "/" {
        ("200 OK", handler.handle_json(&body))
    } else {
        ("404 Not Found", b"not found".to_vec())
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        response.len()
    )
    .and_then(|_| stream.write_all(&response))
    .map_err(|error| BackendError::Rpc(error.to_string()))
}

const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;

fn read_http_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>), BackendError> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let header_end;
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| BackendError::Rpc(error.to_string()))?;
        if read == 0 {
            return Err(BackendError::Rpc("truncated HTTP request".into()));
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_HTTP_BYTES {
            return Err(BackendError::Rpc("HTTP request too large".into()));
        }
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|error| BackendError::Rpc(error.to_string()))?;
    let mut lines = header.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| BackendError::Rpc("missing HTTP request line".into()))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| BackendError::Rpc("missing HTTP method".into()))?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or_else(|| BackendError::Rpc("missing HTTP path".into()))?
        .to_owned();
    let length = lines
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    if length > MAX_HTTP_BYTES || header_end.checked_add(length).is_none() {
        return Err(BackendError::Rpc("HTTP request too large".into()));
    }
    while bytes.len() < header_end + length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| BackendError::Rpc(error.to_string()))?;
        if read == 0 {
            return Err(BackendError::Rpc("truncated HTTP body".into()));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok((
        method,
        path,
        bytes[header_end..header_end + length].to_vec(),
    ))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn service_json(record: &ServiceRecord) -> Result<Value, BackendError> {
    Ok(json!({
        "serviceId": record.service_id,
        "serviceKey": hash_hex(record.service_key.as_bytes()),
        "codeHash": hash_hex(&record.code_hash),
        "abiVersion": record.abi_version,
        "plannerArtifact": {
            "digest": hash_hex(&record.application_artifact.digest),
            "locator": record.application_artifact.locator,
        },
    }))
}

fn parse_service_record(params: &Value) -> Result<ServiceRecord, BackendError> {
    let artifact = params
        .get("plannerArtifact")
        .ok_or_else(|| BackendError::Rpc("plannerArtifact is required".into()))?;
    let digest = if let Some(value) = artifact
        .as_object()
        .and_then(|object| object.get("digest"))
        .and_then(Value::as_str)
    {
        parse_hash(value)?
    } else if let Some(value) = params.get("plannerArtifactDigest").and_then(Value::as_str) {
        parse_hash(value)?
    } else {
        return Err(BackendError::Rpc(
            "plannerArtifact.digest is required".into(),
        ));
    };
    let locator = artifact
        .as_object()
        .and_then(|object| object.get("locator"))
        .and_then(Value::as_str)
        .or_else(|| artifact.as_str())
        .ok_or_else(|| BackendError::Rpc("plannerArtifact.locator is required".into()))?;
    Ok(ServiceRecord {
        service_id: required_u32(params, "serviceId")?,
        service_key: ServiceKeyV1::new(parse_hash(required_str(params, "serviceKey")?)?),
        code_hash: parse_hash(required_str(params, "codeHash")?)?,
        abi_version: required_u32(params, "abiVersion")?,
        application_artifact: ApplicationArtifactRef {
            digest,
            locator: locator.into(),
        },
        deployment: DeploymentMetadata {
            manifest_digest: params
                .get("manifestDigest")
                .and_then(Value::as_str)
                .map(parse_hash)
                .transpose()?,
            registered_at: params.get("registeredAt").and_then(Value::as_u64),
        },
    })
}

fn required_str<'a>(params: &'a Value, name: &str) -> Result<&'a str, BackendError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| BackendError::Rpc(format!("{name} is required")))
}

fn required_u32(params: &Value, name: &str) -> Result<u32, BackendError> {
    params
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| BackendError::Rpc(format!("{name} must be a u32")))
}

fn parse_hash(value: &str) -> Result<StateRoot, BackendError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() != 64 {
        return Err(BackendError::Rpc(
            "expected a 32-byte hexadecimal value".into(),
        ));
    }
    let mut output = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(output)
}

fn nibble(value: u8) -> Result<u8, BackendError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(BackendError::Rpc("invalid hexadecimal value".into())),
    }
}

fn hash_hex(bytes: &[u8]) -> String {
    let mut value = String::from("0x");
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

fn query_json(service_id: u32, response: &StateQueryResponseV1) -> Result<Value, BackendError> {
    Ok(json!({
        "serviceId": service_id,
        "stateRoot": hash_hex(&response.state_root),
        "keyBase64": BASE64.encode(&response.key),
        "valueBase64": response.value.as_ref().map(|value| BASE64.encode(value)),
        "proofBase64": response.proof.iter().map(|node| BASE64.encode(node)).collect::<Vec<_>>(),
    }))
}

fn rpc_error(error: &BackendError) -> Value {
    let (code, message) = match error {
        BackendError::Rpc(message) => (-32000, message.clone()),
        BackendError::Registry(_) => (-32011, "unknown or invalid Service".into()),
        BackendError::Provider(_) => (-32030, "managed-state root is unavailable".into()),
        _ => (-32030, format!("backend request failed: {error:?}")),
    };
    json!({"code": code, "message": message})
}

#[derive(Clone, Default)]
pub struct BackendState {
    pub registry: ServiceRegistry,
    pub applications: ApplicationRegistry,
    pub provider: FullStateProvider,
    pending: BTreeMap<WorkKey, RuntimeRefineOutputV1>,
    predictions: BTreeMap<WorkKey, RuntimeRefineOutputV1>,
}

impl BackendState {
    pub fn register(&mut self, record: ServiceRecord) -> Result<(), BackendError> {
        let service_id = record.service_id;
        let changed_artifact = self
            .registry
            .get(service_id)
            .map(|existing| existing.application_artifact != record.application_artifact)
            .unwrap_or(false);
        self.registry
            .register(record)
            .map_err(BackendError::Registry)?;
        if changed_artifact {
            self.applications.remove(service_id);
        }
        Ok(())
    }

    pub fn register_planner(
        &mut self,
        service_id: u32,
        planner: Arc<dyn ApplicationPlanner>,
    ) -> Result<(), BackendError> {
        let service = self.registry.get(service_id)?;
        self.applications.bind(service, planner)
    }

    pub fn plan(
        &self,
        service_id: u32,
        request: &PlannerRequest,
    ) -> Result<PlannerResult, BackendError> {
        self.applications
            .get(service_id)?
            .plan(request)
            .map_err(BackendError::Planner)
    }

    pub fn application(&self, service_id: u32) -> Result<RegisteredApplication<'_>, BackendError> {
        Ok(RegisteredApplication::new(
            self.applications.get(service_id)?.as_ref(),
        ))
    }

    pub fn track_prediction(
        &mut self,
        service_id: u32,
        package_hash: StateRoot,
        output: RuntimeRefineOutputV1,
    ) -> Result<WorkKey, BackendError> {
        self.registry.get(service_id)?;
        let key = WorkKey {
            service_id,
            package_hash,
        };
        self.pending.insert(key, output.clone());
        self.predictions.insert(key, output);
        Ok(key)
    }

    pub fn prediction(
        &self,
        service_id: u32,
        package_hash: StateRoot,
    ) -> Option<&RuntimeRefineOutputV1> {
        self.predictions.get(&WorkKey {
            service_id,
            package_hash,
        })
    }

    pub fn materialize_if_canonical(
        &mut self,
        key: WorkKey,
        canonical_root: StateRoot,
    ) -> Result<bool, BackendError> {
        let Some(output) = self.pending.get(&key).cloned() else {
            return Ok(false);
        };
        if canonical_root != output.new_root {
            return Ok(false);
        }
        let record = self.registry.get(key.service_id)?;
        self.provider
            .apply_recovery(record.service_key, &output)
            .map_err(BackendError::Provider)?;
        self.pending.remove(&key);
        Ok(true)
    }

    pub fn apply_recovery(&mut self, envelope: RecoveryEnvelopeV1) -> Result<(), BackendError> {
        let record = self
            .registry
            .get(envelope.service_id)
            .map_err(|_| BackendError::UnknownService)?;
        if record.service_key != envelope.service_key {
            return Err(BackendError::ServiceKeyMismatch);
        }
        self.provider
            .apply_recovery(record.service_key, &envelope.output)
            .map_err(BackendError::Provider)
    }

    pub fn append_recovery(
        &self,
        path: &Path,
        envelope: &RecoveryEnvelopeV1,
    ) -> Result<(), BackendError> {
        let encoded = envelope.encode()?;
        let length = u32::try_from(encoded.len()).map_err(|_| BackendError::EntryTooLarge)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| BackendError::InvalidEnvelope)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|_| BackendError::InvalidEnvelope)?;
        file.write_all(&length.to_le_bytes())
            .and_then(|_| file.write_all(&encoded))
            .and_then(|_| file.sync_data())
            .map_err(|_| BackendError::InvalidEnvelope)
    }

    pub fn replay_recovery_log(&mut self, path: &Path) -> Result<(), BackendError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(BackendError::InvalidEnvelope),
        };
        let mut offset = 0usize;
        while offset < bytes.len() {
            let length_bytes = bytes
                .get(offset..offset + 4)
                .ok_or(BackendError::TruncatedLog)?;
            let length = u32::from_le_bytes(
                length_bytes
                    .try_into()
                    .map_err(|_| BackendError::TruncatedLog)?,
            ) as usize;
            offset += 4;
            if length > MAX_RECOVERY_BYTES + 128 * 1024 {
                return Err(BackendError::EntryTooLarge);
            }
            let entry = bytes
                .get(offset..offset + length)
                .ok_or(BackendError::TruncatedLog)?;
            offset += length;
            let envelope = RecoveryEnvelopeV1::decode(entry)?;
            self.apply_recovery(envelope)?;
        }
        Ok(())
    }
}

struct EnvelopeReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> EnvelopeReader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], BackendError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(BackendError::InvalidEnvelope)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(BackendError::TruncatedLog)?;
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, BackendError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, BackendError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| BackendError::InvalidEnvelope)?,
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BackendError> {
        self.take(N)?
            .try_into()
            .map_err(|_| BackendError::InvalidEnvelope)
    }

    fn bytes(&mut self, maximum: usize) -> Result<Vec<u8>, BackendError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(BackendError::EntryTooLarge);
        }
        Ok(self.take(length)?.to_vec())
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_runtime_core::{ActionReceiptV1, ActionStatusV1, StateChangeV1, StateDiffV1};
    use service_runtime_host::MaterializedServiceStateProvider;
    use service_runtime_state::FullState;
    use std::sync::Arc;

    fn record(id: u32, key: u8) -> ServiceRecord {
        ServiceRecord {
            service_id: id,
            service_key: ServiceKeyV1::new([key; 32]),
            code_hash: [key + 1; 32],
            abi_version: 1,
            application_artifact: ApplicationArtifactRef {
                digest: [key + 2; 32],
                locator: format!("artifact-{id}"),
            },
            deployment: DeploymentMetadata::default(),
        }
    }

    fn output() -> RuntimeRefineOutputV1 {
        RuntimeRefineOutputV1::from_diff(
            [1; 32],
            [2; 32],
            vec![ActionReceiptV1 {
                action_hash: [3; 32],
                status: ActionStatusV1::Applied,
                error_code: None,
            }],
            StateDiffV1::default(),
        )
        .unwrap()
    }

    struct TestPlanner(StateRoot, Vec<u8>);

    impl ApplicationPlanner for TestPlanner {
        fn artifact_digest(&self) -> StateRoot {
            self.0
        }

        fn plan(&self, _request: &PlannerRequest) -> Result<PlannerResult, PlannerError> {
            Ok(PlannerResult {
                local_access_keys: vec![self.1.clone()],
                external_access_keys: BTreeMap::new(),
            })
        }
    }

    struct TestLoader;

    impl ApplicationArtifactLoader for TestLoader {
        fn load(
            &self,
            artifact: &ApplicationArtifactRef,
        ) -> Result<Arc<dyn ApplicationPlanner>, PlannerError> {
            let key = if artifact.locator.ends_with("10") {
                b"a".to_vec()
            } else {
                b"b".to_vec()
            };
            Ok(Arc::new(TestPlanner(artifact.digest, key)))
        }
    }

    #[test]
    fn registry_enforces_bijective_identity_but_allows_code_upgrade() {
        let mut registry = ServiceRegistry::default();
        registry.register(record(10, 1)).unwrap();
        let mut upgraded = record(10, 1);
        upgraded.code_hash = [9; 32];
        registry.register(upgraded).unwrap();
        assert_eq!(registry.get(10).unwrap().code_hash, [9; 32]);
        assert_eq!(
            registry.register(record(10, 2)),
            Err(RegistryError::ServiceIdAlreadyBound)
        );
        assert_eq!(
            registry.register(record(11, 1)),
            Err(RegistryError::ServiceKeyAlreadyBound)
        );
    }

    #[test]
    fn same_package_hash_isolated_by_service_id() {
        let mut state = BackendState::default();
        state.register(record(10, 1)).unwrap();
        state.register(record(11, 2)).unwrap();
        let output = output();
        let package = [7; 32];
        assert_eq!(
            state
                .track_prediction(10, package, output.clone())
                .unwrap()
                .service_id,
            10
        );
        assert_eq!(
            state
                .track_prediction(11, package, output)
                .unwrap()
                .service_id,
            11
        );
        assert!(state.prediction(10, package).is_some());
        assert!(state.prediction(11, package).is_some());
    }

    #[test]
    fn one_backend_routes_distinct_planner_artifacts_without_rebuild() {
        let mut state = BackendState::default();
        state.register(record(10, 1)).unwrap();
        state.register(record(11, 2)).unwrap();
        let a_digest = state.registry.get(10).unwrap().application_artifact.digest;
        let b_digest = state.registry.get(11).unwrap().application_artifact.digest;
        state
            .register_planner(10, Arc::new(TestPlanner(a_digest, b"a".to_vec())))
            .unwrap();
        state
            .register_planner(11, Arc::new(TestPlanner(b_digest, b"b".to_vec())))
            .unwrap();
        assert_eq!(
            state
                .plan(10, &PlannerRequest { actions: vec![] })
                .unwrap()
                .local_access_keys,
            vec![b"a".to_vec()]
        );
        assert_eq!(
            state
                .plan(11, &PlannerRequest { actions: vec![] })
                .unwrap()
                .local_access_keys,
            vec![b"b".to_vec()]
        );
    }

    #[test]
    fn recovery_envelope_is_strict_and_service_scoped() {
        let envelope = RecoveryEnvelopeV1 {
            service_id: 10,
            service_key: ServiceKeyV1::new([1; 32]),
            output: output(),
        };
        let encoded = envelope.encode().unwrap();
        assert_eq!(RecoveryEnvelopeV1::decode(&encoded).unwrap(), envelope);
        assert!(matches!(
            RecoveryEnvelopeV1::decode(&encoded[..encoded.len() - 1]),
            Err(BackendError::TruncatedLog) | Err(BackendError::Wire(_))
        ));
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert_eq!(
            RecoveryEnvelopeV1::decode(&trailing),
            Err(BackendError::InvalidEnvelope)
        );
    }

    #[test]
    fn public_rpc_exposes_capabilities_and_restricts_registration() {
        let handler =
            BackendRpcHandler::new(BackendState::default(), Arc::new(UnconfiguredWorkGateway))
                .with_admin_token("secret");
        let capabilities = handler
            .handle("jamscript_getCapabilitiesV1", Value::Null)
            .unwrap();
        assert_eq!(capabilities["protocolVersion"], 1);
        assert_eq!(capabilities["multiService"], true);

        let registration = json!({
            "serviceId": 10,
            "serviceKey": hash_hex([1; 32].as_slice()),
            "codeHash": hash_hex([2; 32].as_slice()),
            "abiVersion": 1,
            "plannerArtifact": {
                "digest": hash_hex([3; 32].as_slice()),
                "locator": "artifact-10"
            }
        });
        assert!(handler
            .handle("jamscript_registerServiceV1", registration.clone())
            .is_err());
        let mut authorized = registration.as_object().unwrap().clone();
        authorized.insert("adminToken".into(), Value::String("secret".into()));
        handler
            .handle("jamscript_registerServiceV1", Value::Object(authorized))
            .unwrap();
        let services = handler
            .handle("jamscript_listServicesV1", Value::Null)
            .unwrap();
        assert_eq!(services.as_array().unwrap().len(), 1);
    }

    #[test]
    fn json_rpc_handler_returns_protocol_errors_without_panicking() {
        let handler =
            BackendRpcHandler::new(BackendState::default(), Arc::new(UnconfiguredWorkGateway));
        let response = handler.handle_json(
            br#"{"jsonrpc":"2.0","id":7,"method":"jamscript_getCapabilitiesV1","params":{}}"#,
        );
        let response: Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"]["managedStateVersion"], 1);

        let invalid = handler.handle_json(b"not json");
        let invalid: Value = serde_json::from_slice(&invalid).unwrap();
        assert_eq!(invalid["error"]["code"], -32600);
    }

    #[test]
    fn registration_loads_service_artifacts_into_one_runtime() {
        let handler =
            BackendRpcHandler::new(BackendState::default(), Arc::new(UnconfiguredWorkGateway))
                .with_admin_token("secret")
                .with_artifact_loader(Arc::new(TestLoader));
        for (id, key) in [(10, 1), (11, 2)] {
            let record = record(id, key);
            let params = json!({
                "adminToken": "secret",
                "serviceId": record.service_id,
                "serviceKey": hash_hex(record.service_key.as_bytes()),
                "codeHash": hash_hex(&record.code_hash),
                "abiVersion": record.abi_version,
                "plannerArtifact": {
                    "digest": hash_hex(&record.application_artifact.digest),
                    "locator": record.application_artifact.locator,
                }
            });
            handler
                .handle("jamscript_registerServiceV1", params)
                .unwrap();
        }
        let state = handler.state.lock().unwrap();
        assert_eq!(
            state
                .plan(10, &PlannerRequest { actions: vec![] })
                .unwrap()
                .local_access_keys,
            vec![b"a".to_vec()]
        );
        assert_eq!(
            state
                .plan(11, &PlannerRequest { actions: vec![] })
                .unwrap()
                .local_access_keys,
            vec![b"b".to_vec()]
        );
    }

    #[test]
    fn recovery_log_replays_isolated_service_snapshots_after_restart() {
        let service_a = record(10, 1);
        let service_b = record(11, 2);
        let diff_a = StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: b"a".to_vec(),
                value: Some(b"A".to_vec()),
            }],
        };
        let diff_b = StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: b"b".to_vec(),
                value: Some(b"B".to_vec()),
            }],
        };
        let root_a = FullState::empty().apply_diff(&diff_a).unwrap().root();
        let root_b = FullState::empty().apply_diff(&diff_b).unwrap().root();
        let output_a = RuntimeRefineOutputV1::from_diff(
            service_runtime_core::EMPTY_STATE_ROOT_V1,
            root_a,
            Vec::new(),
            diff_a,
        )
        .unwrap();
        let output_b = RuntimeRefineOutputV1::from_diff(
            service_runtime_core::EMPTY_STATE_ROOT_V1,
            root_b,
            Vec::new(),
            diff_b,
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.log");
        let mut state = BackendState::default();
        state.register(service_a.clone()).unwrap();
        state.register(service_b.clone()).unwrap();
        state
            .append_recovery(
                &path,
                &RecoveryEnvelopeV1 {
                    service_id: service_a.service_id,
                    service_key: service_a.service_key,
                    output: output_a,
                },
            )
            .unwrap();
        state
            .append_recovery(
                &path,
                &RecoveryEnvelopeV1 {
                    service_id: service_b.service_id,
                    service_key: service_b.service_key,
                    output: output_b,
                },
            )
            .unwrap();

        let mut restarted = BackendState::default();
        restarted.register(service_a.clone()).unwrap();
        restarted.register(service_b.clone()).unwrap();
        restarted.replay_recovery_log(&path).unwrap();
        assert_eq!(
            restarted
                .provider
                .materialized_root(service_a.service_key)
                .unwrap(),
            root_a
        );
        assert_eq!(
            restarted
                .provider
                .materialized_root(service_b.service_key)
                .unwrap(),
            root_b
        );
    }
}
