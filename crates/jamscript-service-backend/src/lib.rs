//! Multi-Service backend state and identity boundaries.
//!
//! The backend owns routing and materialized state, while JAM remains the
//! authority for canonical commitments.  This crate intentionally does not
//! embed a generated application: artifacts are registered per Service and a
//! daemon can route many Services without being rebuilt.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use jam_program_blob_common::ProgramBlob;
use jamscript_deployment::JsonRpcTransport;
use polkavm::{
    BackendKind, Config as PvmConfig, Engine, Linker, MemoryAccessError, Module, ModuleConfig, Reg,
};
use serde_json::{json, Value};
use service_runtime_core::{
    BackendMetadataV1, ExecutionContext, RuntimeRefineInputV1, RuntimeRefineOutputV1,
    ServiceApplication, ServiceKeyV1, StateAccessError, StateAccessPlanV1, StateQueryResponseV1,
    StateRoot, WireError, EMPTY_STATE_ROOT_V1, MAX_RECOVERY_BYTES, RECOVERY_FORMAT_VERSION,
};
use service_runtime_host::{
    FullStateProvider, MaterializedServiceStateProvider, ProviderError, ServiceStateProvider,
};
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const BACKEND_PROTOCOL_VERSION_V1: u32 = 1;
pub const MAX_PVM_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactFormat {
    JamScriptPvmV1,
}

impl ArtifactFormat {
    pub const WIRE_NAME: &'static str = "jamscript-pvm-v1";

    fn parse(value: &str) -> Result<Self, BackendError> {
        (value == Self::WIRE_NAME)
            .then_some(Self::JamScriptPvmV1)
            .ok_or_else(|| BackendError::Rpc("unsupported artifact format".into()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationArtifactRef {
    pub digest: StateRoot,
    pub format: ArtifactFormat,
}

/// Content-addressed artifact storage. The backend only accepts bytes after
/// verifying their digest and re-verifies persisted bytes on every load so a
/// damaged volume cannot silently select a different Service program.
pub trait ArtifactStore: Send + Sync {
    fn put_verified(&self, digest: StateRoot, bytes: &[u8]) -> Result<usize, BackendError>;
    fn get_verified(&self, digest: StateRoot) -> Result<Vec<u8>, BackendError>;
    fn contains(&self, digest: StateRoot) -> Result<bool, BackendError>;
    fn root(&self) -> Option<&Path> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct DiskArtifactStore {
    root: PathBuf,
}

impl DiskArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
        Ok(Self { root })
    }

    fn path(&self, digest: &StateRoot) -> PathBuf {
        self.root.join(hash_hex(digest)).with_extension("pvm")
    }
}

impl ArtifactStore for DiskArtifactStore {
    fn put_verified(&self, digest: StateRoot, bytes: &[u8]) -> Result<usize, BackendError> {
        if bytes.is_empty() || bytes.len() > MAX_PVM_ARTIFACT_BYTES {
            return Err(BackendError::ArtifactTooLarge);
        }
        if service_runtime_core::blake2_256(bytes) != digest {
            return Err(BackendError::ArtifactDigestMismatch);
        }
        let target = self.path(&digest);
        if target.exists() {
            let existing = fs::read(&target)
                .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
            if service_runtime_core::blake2_256(&existing) != digest {
                return Err(BackendError::ArtifactCorrupt);
            }
            return Ok(existing.len());
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp = self
            .root
            .join(format!(".{}.{}.tmp", hash_hex(&digest), stamp));
        let result = (|| -> Result<usize, BackendError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
            match fs::rename(&temp, &target) {
                Ok(()) => Ok(bytes.len()),
                Err(_error) if target.exists() => {
                    let existing = fs::read(&target).map_err(|read_error| {
                        BackendError::ArtifactStore(read_error.to_string())
                    })?;
                    if service_runtime_core::blake2_256(&existing) != digest {
                        return Err(BackendError::ArtifactCorrupt);
                    }
                    Ok(existing.len())
                }
                Err(error) => Err(BackendError::ArtifactStore(error.to_string())),
            }
        })();
        let _ = fs::remove_file(&temp);
        result
    }

    fn get_verified(&self, digest: StateRoot) -> Result<Vec<u8>, BackendError> {
        let bytes = fs::read(self.path(&digest)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                BackendError::ArtifactNotFound
            } else {
                BackendError::ArtifactStore(error.to_string())
            }
        })?;
        if bytes.len() > MAX_PVM_ARTIFACT_BYTES {
            return Err(BackendError::ArtifactTooLarge);
        }
        if service_runtime_core::blake2_256(&bytes) != digest {
            return Err(BackendError::ArtifactCorrupt);
        }
        Ok(bytes)
    }

    fn contains(&self, digest: StateRoot) -> Result<bool, BackendError> {
        match self.get_verified(digest) {
            Ok(_) => Ok(true),
            Err(BackendError::ArtifactNotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

#[derive(Clone, Debug)]
pub struct DiskBackendStore {
    root: PathBuf,
    registry_path: PathBuf,
    recovery_path: PathBuf,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedServiceRecord {
    service_id: u32,
    service_key: String,
    code_hash: String,
    abi_version: u32,
    artifact_digest: String,
    artifact_format: String,
    manifest_digest: Option<String>,
    registered_at: Option<u64>,
}

impl DiskBackendStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
        Ok(Self {
            registry_path: root.join("registry.json"),
            recovery_path: root.join("recovery.log"),
            root,
        })
    }

    pub fn artifact_store(&self) -> Result<DiskArtifactStore, BackendError> {
        DiskArtifactStore::new(self.root.join("artifacts"))
    }

    pub fn recovery_path(&self) -> &Path {
        &self.recovery_path
    }

    pub fn load_registry(&self) -> Result<ServiceRegistry, BackendError> {
        let bytes = match fs::read(&self.registry_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ServiceRegistry::default())
            }
            Err(error) => return Err(BackendError::ArtifactStore(error.to_string())),
        };
        let records: Vec<PersistedServiceRecord> = serde_json::from_slice(&bytes)
            .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
        let mut registry = ServiceRegistry::default();
        for record in records {
            registry.register(ServiceRecord {
                service_id: record.service_id,
                service_key: ServiceKeyV1::new(parse_hash(&record.service_key)?),
                code_hash: parse_hash(&record.code_hash)?,
                abi_version: record.abi_version,
                application_artifact: ApplicationArtifactRef {
                    digest: parse_hash(&record.artifact_digest)?,
                    format: ArtifactFormat::parse(&record.artifact_format)?,
                },
                deployment: DeploymentMetadata {
                    manifest_digest: record
                        .manifest_digest
                        .as_deref()
                        .map(parse_hash)
                        .transpose()?,
                    registered_at: record.registered_at,
                },
            })?;
        }
        Ok(registry)
    }

    pub fn persist_registry(&self, registry: &ServiceRegistry) -> Result<(), BackendError> {
        let records = registry
            .iter()
            .map(|record| PersistedServiceRecord {
                service_id: record.service_id,
                service_key: hash_hex(record.service_key.as_bytes()),
                code_hash: hash_hex(&record.code_hash),
                abi_version: record.abi_version,
                artifact_digest: hash_hex(&record.application_artifact.digest),
                artifact_format: ArtifactFormat::WIRE_NAME.into(),
                manifest_digest: record
                    .deployment
                    .manifest_digest
                    .map(|hash| hash_hex(&hash)),
                registered_at: record.deployment.registered_at,
            })
            .collect::<Vec<_>>();
        let bytes = serde_json::to_vec_pretty(&records)
            .map_err(|error| BackendError::ArtifactStore(error.to_string()))?;
        let temp = self.registry_path.with_extension("json.tmp");
        fs::write(&temp, bytes)
            .and_then(|_| fs::rename(&temp, &self.registry_path))
            .map_err(|error| BackendError::ArtifactStore(error.to_string()))
    }
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
/// daemon. Production uses the linked PVM implementation below.
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

/// A loader turns a registered portable PVM artifact into a planner/runtime
/// binding without compiling the backend daemon for a new Service.
pub trait ApplicationArtifactLoader: Send + Sync {
    fn load(
        &self,
        artifact: &ApplicationArtifactRef,
    ) -> Result<Arc<dyn ApplicationPlanner>, PlannerError>;

    fn metadata(
        &self,
        _artifact: &ApplicationArtifactRef,
    ) -> Result<Option<BackendMetadataV1>, PlannerError> {
        Ok(None)
    }

    fn canonical_code_hash(
        &self,
        _artifact: &ApplicationArtifactRef,
    ) -> Result<Option<StateRoot>, PlannerError> {
        Ok(None)
    }
}

/// The only production artifact loader. It executes linked PolkaVM bytes
/// directly and never invokes rustc, Cargo, LLVM, ScriptC, or a generated
/// source file at request time.
#[derive(Clone)]
pub struct PvmArtifactLoader {
    store: Arc<dyn ArtifactStore>,
}

impl PvmArtifactLoader {
    pub fn new(store: Arc<dyn ArtifactStore>) -> Self {
        Self { store }
    }

    pub fn load_pvm(
        &self,
        artifact: &ApplicationArtifactRef,
    ) -> Result<Arc<PvmApplication>, BackendError> {
        let bytes = self.store.get_verified(artifact.digest)?;
        let canonical_code_hash = canonical_pvm_code_hash(&bytes)?;
        let application = PvmApplication {
            artifact_digest: artifact.digest,
            bytes: Arc::new(bytes),
            canonical_code_hash,
        };
        Ok(Arc::new(application))
    }
}

impl ApplicationArtifactLoader for PvmArtifactLoader {
    fn load(
        &self,
        artifact: &ApplicationArtifactRef,
    ) -> Result<Arc<dyn ApplicationPlanner>, PlannerError> {
        if artifact.format != ArtifactFormat::JamScriptPvmV1 {
            return Err(PlannerError::InvalidArtifact);
        }
        self.load_pvm(artifact)
            .map(|application| application as Arc<dyn ApplicationPlanner>)
            .map_err(|_| PlannerError::InvalidArtifact)
    }

    fn metadata(
        &self,
        artifact: &ApplicationArtifactRef,
    ) -> Result<Option<BackendMetadataV1>, PlannerError> {
        self.load_pvm(artifact)
            .and_then(|application| application.metadata())
            .map(Some)
            .map_err(|_| PlannerError::InvalidArtifact)
    }

    fn canonical_code_hash(
        &self,
        artifact: &ApplicationArtifactRef,
    ) -> Result<Option<StateRoot>, PlannerError> {
        self.load_pvm(artifact)
            .map(|application| Some(application.canonical_code_hash()))
            .map_err(|_| PlannerError::InvalidArtifact)
    }
}

pub struct PvmApplication {
    artifact_digest: StateRoot,
    bytes: Arc<Vec<u8>>,
    pub canonical_code_hash: StateRoot,
}

impl PvmApplication {
    pub fn canonical_code_hash(&self) -> StateRoot {
        self.canonical_code_hash
    }

    pub fn metadata(&self) -> Result<BackendMetadataV1, BackendError> {
        let bytes = self.invoke("jamscript_backend_metadata_v1", &[])?;
        BackendMetadataV1::decode(&bytes).map_err(BackendError::Wire)
    }

    pub fn plan_encoded(&self, input: &[u8]) -> Result<Vec<u8>, BackendError> {
        self.invoke("jamscript_plan_v1", input)
    }

    pub fn plan_with_keys(
        &self,
        actions: &[Vec<u8>],
        keys: &[Vec<u8>],
    ) -> Result<PlannerResult, BackendError> {
        let plan = StateAccessPlanV1::from_keys(keys).map_err(BackendError::Wire)?;
        self.plan_with_witness(
            actions,
            service_runtime_core::ManagedStateWitnessV1 {
                version: service_runtime_core::ManagedStateWitnessV1::VERSION,
                parent_root: EMPTY_STATE_ROOT_V1,
                access_plan: plan,
                storage_proof: Vec::new(),
            },
        )
    }

    pub fn plan_with_witness(
        &self,
        actions: &[Vec<u8>],
        managed_state: service_runtime_core::ManagedStateWitnessV1,
    ) -> Result<PlannerResult, BackendError> {
        self.plan_with_external_witness(actions, managed_state, Vec::new())
    }

    pub fn plan_with_external_witness(
        &self,
        actions: &[Vec<u8>],
        managed_state: service_runtime_core::ManagedStateWitnessV1,
        external_state: Vec<service_runtime_core::ExternalStateWitnessV1>,
    ) -> Result<PlannerResult, BackendError> {
        let input = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state,
            external_state,
            actions: actions.to_vec(),
        };
        let output = self.plan_encoded(&input.encode().map_err(BackendError::Wire)?)?;
        if let Ok((service_id, key)) =
            service_runtime_core::decode_planner_need_external_state(&output)
        {
            return Ok(PlannerResult {
                local_access_keys: Vec::new(),
                external_access_keys: BTreeMap::from([(service_id, vec![key])]),
            });
        }
        if let Ok(key) = service_runtime_core::decode_planner_need_state(&output) {
            return Ok(PlannerResult {
                local_access_keys: vec![key],
                external_access_keys: BTreeMap::new(),
            });
        }
        if output == [0] || RuntimeRefineOutputV1::decode(&output).is_ok() {
            return Ok(PlannerResult::default());
        }
        Err(BackendError::Planner(PlannerError::ApplicationRejected))
    }

    pub fn refine_encoded(&self, input: &[u8]) -> Result<RuntimeRefineOutputV1, BackendError> {
        let bytes = self.invoke("minijam_refine", input)?;
        RuntimeRefineOutputV1::decode(&bytes).map_err(BackendError::Wire)
    }

    fn invoke(&self, export: &str, payload: &[u8]) -> Result<Vec<u8>, BackendError> {
        let mut config = PvmConfig::new();
        config.set_backend(Some(BackendKind::Interpreter));
        let engine = Engine::new(&config).map_err(|error| BackendError::Pvm(error.to_string()))?;
        let module = Module::new(
            &engine,
            &ModuleConfig::new(),
            self.bytes.as_ref().clone().into(),
        )
        .map_err(|error| BackendError::Pvm(error.to_string()))?;
        let payload = payload.to_vec();
        let mut linker: Linker<(), MemoryAccessError> = Linker::new();
        linker
            .define_typed(
                "minijam_host_call",
                move |caller: polkavm::Caller<'_, ()>,
                      call: u32,
                      args: u32|
                      -> Result<u64, MemoryAccessError> {
                    let raw = caller.instance.read_memory(args, 48)?;
                    let mut values = [0u64; 6];
                    for (index, value) in values.iter_mut().enumerate() {
                        let offset = index * 8;
                        *value = u64::from_le_bytes(
                            raw[offset..offset + 8].try_into().expect("fixed host args"),
                        );
                    }
                    if call == 1 && values[3] == 13 {
                        caller.instance.write_memory(values[0] as u32, &payload)?;
                        return Ok(payload.len() as u64);
                    }
                    Ok(u64::MAX)
                },
            )
            .map_err(|error| BackendError::Pvm(error.to_string()))?;
        linker.define_fallback(|caller, _| {
            caller.instance.set_reg(Reg::A0, u64::MAX);
            Ok(())
        });
        let pre = linker
            .instantiate_pre(&module)
            .map_err(|error| BackendError::Pvm(error.to_string()))?;
        let mut instance = pre
            .instantiate()
            .map_err(|error| BackendError::Pvm(error.to_string()))?;
        instance.set_gas(5_000_000);
        instance
            .call_typed_and_get_result::<u64, _>(&mut (), export, ())
            .map_err(|error| BackendError::Pvm(format!("{export}: {error:?}")))?;
        let pointer = instance.reg(Reg::A0) as u32;
        let size = instance.reg(Reg::A1) as u32;
        if size as usize > MAX_RECOVERY_BYTES + MAX_PVM_ARTIFACT_BYTES.min(2 * 1024 * 1024) {
            return Err(BackendError::EntryTooLarge);
        }
        instance
            .read_memory(pointer, size)
            .map_err(|error| BackendError::Pvm(error.to_string()))
    }
}

impl ApplicationPlanner for PvmApplication {
    fn artifact_digest(&self) -> StateRoot {
        self.artifact_digest
    }

    fn plan(&self, request: &PlannerRequest) -> Result<PlannerResult, PlannerError> {
        self.plan_with_keys(&request.actions, &[])
            .map_err(|_| PlannerError::ApplicationRejected)
    }
}

fn canonical_pvm_code_hash(bytes: &[u8]) -> Result<StateRoot, BackendError> {
    let parts = polkavm_linker::ProgramParts::from_bytes(bytes.to_vec().into())
        .map_err(|error| BackendError::Pvm(error.to_string()))?;
    let blob = ProgramBlob::from_pvm(&parts, std::borrow::Cow::Borrowed(&[]))
        .to_vec()
        .map_err(|error| BackendError::Pvm(error.to_string()))?;
    Ok(service_runtime_core::blake2_256(&blob))
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
    ArtifactNotFound,
    ArtifactTooLarge,
    ArtifactDigestMismatch,
    ArtifactCorrupt,
    ArtifactStore(String),
    Pvm(String),
    CodeHashMismatch,
    InvalidMetadata,
    StaleContext,
    PredictionStale,
    Rpc(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for BackendError {}

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
    pub dynamic_pvm_services: bool,
}

impl Default for CapabilitiesV1 {
    fn default() -> Self {
        Self {
            protocol_version: BACKEND_PROTOCOL_VERSION_V1,
            managed_state_version: 1,
            multi_service: true,
            external_state_witness: true,
            dynamic_pvm_services: false,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedContextV1 {
    pub block_hash: StateRoot,
    pub block_number: u32,
    pub state_root: StateRoot,
    pub slot: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainServiceInfoV1 {
    pub service_id: u32,
    pub code_hash: StateRoot,
}

/// Network boundary used by the production engine. The backend never reads
/// node or Formal RPCs from request handlers directly; this interface keeps
/// finality, preimages, storage, and work submission in one auditable layer.
pub trait BackendNetwork: Send + Sync {
    fn genesis_hash(&self) -> Result<StateRoot, BackendError>;
    fn finalized_context(&self) -> Result<FinalizedContextV1, BackendError>;
    fn service_info(
        &self,
        context: &FinalizedContextV1,
        service_id: u32,
    ) -> Result<Option<ChainServiceInfoV1>, BackendError>;
    fn service_storage_at(
        &self,
        context: &FinalizedContextV1,
        service_id: u32,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, BackendError>;
    fn service_code(
        &self,
        context: &FinalizedContextV1,
        service_id: u32,
        code_hash: StateRoot,
    ) -> Result<Option<Vec<u8>>, BackendError>;
    fn submit_work(&self, params: Value) -> Result<Value, BackendError>;
    fn work_status(&self, params: Value) -> Result<Value, BackendError>;
}

pub struct MiniJamNetworkGateway<T> {
    pub transport: T,
    pub node_rpc: String,
    pub formal_rpc: String,
    pub timeout: Duration,
}

/// Consensus-facing orchestration. It discovers the immutable read set with
/// the PVM planner, builds proof witnesses from the service-scoped provider,
/// preflights the exact refine bytes, and only then sends them to Formal.
pub struct BackendEngine {
    network: Arc<dyn BackendNetwork>,
    loader: Arc<PvmArtifactLoader>,
}

impl BackendEngine {
    pub fn new(network: Arc<dyn BackendNetwork>, loader: Arc<PvmArtifactLoader>) -> Self {
        Self { network, loader }
    }

    pub fn submit(&self, state: &mut BackendState, params: Value) -> Result<Value, BackendError> {
        let service_id = required_u32(&params, "serviceId")?;
        let code_hash = parse_hash(required_str(&params, "serviceCodeHash")?)?;
        let record = state.registry.get(service_id)?.clone();
        if record.code_hash != code_hash {
            return Err(BackendError::CodeHashMismatch);
        }
        let action = BASE64
            .decode(required_str(&params, "payloadBase64")?)
            .map_err(|error| BackendError::Rpc(format!("invalid payloadBase64: {error}")))?;
        let context = self.network.finalized_context()?;
        if let Some(request_context) = params.get("context").and_then(Value::as_object) {
            let requested_block = request_context
                .get("blockHash")
                .and_then(Value::as_str)
                .map(parse_hash)
                .transpose()?
                .ok_or(BackendError::StaleContext)?;
            let requested_root = request_context
                .get("stateRoot")
                .and_then(Value::as_str)
                .map(parse_hash)
                .transpose()?
                .ok_or(BackendError::StaleContext)?;
            let requested_slot = request_context
                .get("slot")
                .and_then(Value::as_u64)
                .and_then(|slot| u32::try_from(slot).ok())
                .ok_or(BackendError::StaleContext)?;
            if requested_block != context.block_hash
                || requested_root != context.state_root
                || requested_slot != context.slot
            {
                return Err(BackendError::StaleContext);
            }
        }
        let pvm = self.loader.load_pvm(&record.application_artifact)?;

        let mut keys = Vec::new();
        if let Ok(signed) = jamscript_runtime_core::decode_signed_action_v1(&action) {
            if signed.public_key.len() == 32 {
                let mut sender = [0u8; 32];
                sender.copy_from_slice(signed.public_key);
                keys.push(jamscript_runtime_core::nonce_key(&sender));
            }
        }
        let parent_root = match self.network.service_storage_at(
            &context,
            service_id,
            service_runtime_core::MANAGED_STATE_COMMITMENT_KEY_V1,
        )? {
            Some(bytes) => {
                service_runtime_core::ManagedStateCommitmentV1::decode(&bytes)
                    .map_err(BackendError::Wire)?
                    .root
            }
            None => EMPTY_STATE_ROOT_V1,
        };
        let actions = vec![action.clone()];
        let mut external_keys = BTreeMap::<u32, Vec<Vec<u8>>>::new();
        for _ in 0..64 {
            let plan = StateAccessPlanV1::from_keys(&keys).map_err(BackendError::Wire)?;
            let witness = state
                .provider
                .build_witness(record.service_key, parent_root, &plan)
                .map_err(BackendError::Provider)?;
            let external_witnesses =
                build_external_witnesses(state, self.network.as_ref(), &context, &external_keys)?;
            let planned =
                pvm.plan_with_external_witness(&actions, witness, external_witnesses.clone())?;
            let mut changed = false;
            for key in planned.local_access_keys {
                if !keys.contains(&key) {
                    keys.push(key);
                    changed = true;
                }
            }
            for (external_service_id, discovered_keys) in planned.external_access_keys {
                let keys_for_service = external_keys.entry(external_service_id).or_default();
                for key in discovered_keys {
                    if !keys_for_service.contains(&key) {
                        keys_for_service.push(key);
                        keys_for_service.sort();
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let plan = StateAccessPlanV1::from_keys(&keys).map_err(BackendError::Wire)?;
        let witness = state
            .provider
            .build_witness(record.service_key, parent_root, &plan)
            .map_err(BackendError::Provider)?;
        let external_witnesses =
            build_external_witnesses(state, self.network.as_ref(), &context, &external_keys)?;
        let input = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: witness,
            external_state: external_witnesses,
            actions,
        };
        let encoded = input.encode().map_err(BackendError::Wire)?;
        let prediction = pvm.refine_encoded(&encoded)?;
        if prediction.parent_root != parent_root {
            return Err(BackendError::Rpc(
                "PVM parent root disagrees with chain".into(),
            ));
        }
        let mut forwarded = params;
        forwarded["context"] = json!({
            "blockHash": hash_hex(&context.block_hash),
            "stateRoot": hash_hex(&context.state_root),
            "slot": context.slot,
        });
        forwarded["payloadBase64"] = Value::String(BASE64.encode(&encoded));
        let result = self.network.submit_work(forwarded)?;
        let package_hash = result
            .get("packageHash")
            .and_then(Value::as_str)
            .map(parse_hash)
            .transpose()?
            .ok_or_else(|| BackendError::Rpc("Formal response omitted packageHash".into()))?;
        state.track_prediction(service_id, package_hash, prediction.clone())?;
        let mut response = result;
        if let Some(object) = response.as_object_mut() {
            object.insert("packageHash".into(), Value::String(hash_hex(&package_hash)));
            object.insert(
                "prediction".into(),
                json!({
                    "parentRoot": hash_hex(&prediction.parent_root),
                    "newRoot": hash_hex(&prediction.new_root),
                }),
            );
        }
        Ok(response)
    }

    pub fn status(&self, state: &mut BackendState, params: Value) -> Result<Value, BackendError> {
        let result = self.network.work_status(params.clone())?;
        let Some(package) = params.get("packageHash").and_then(Value::as_str) else {
            return Ok(result);
        };
        let package_hash = parse_hash(package)?;
        let Some(service_id) = params
            .get("serviceId")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
        else {
            return Ok(result);
        };
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "imported" {
            let context = self.network.finalized_context()?;
            let canonical = self
                .network
                .service_storage_at(
                    &context,
                    service_id,
                    service_runtime_core::MANAGED_STATE_COMMITMENT_KEY_V1,
                )?
                .and_then(|bytes| {
                    service_runtime_core::ManagedStateCommitmentV1::decode(&bytes).ok()
                })
                .map(|commitment| commitment.root);
            if let Some(root) = canonical {
                let key = WorkKey {
                    service_id,
                    package_hash,
                };
                let expected = state.prediction(service_id, package_hash).cloned();
                if let Some(output) = expected {
                    if root != output.parent_root && root != output.new_root {
                        state.pending.remove(&key);
                        return Err(BackendError::PredictionStale);
                    }
                }
                state.materialize_if_canonical(key, root)?;
            }
        }
        Ok(result)
    }
}

fn build_external_witnesses(
    state: &BackendState,
    network: &dyn BackendNetwork,
    context: &FinalizedContextV1,
    requested: &BTreeMap<u32, Vec<Vec<u8>>>,
) -> Result<Vec<service_runtime_core::ExternalStateWitnessV1>, BackendError> {
    requested
        .iter()
        .map(|(service_id, keys)| {
            let record = state.registry.get(*service_id)?.clone();
            let commitment = network
                .service_storage_at(
                    context,
                    *service_id,
                    service_runtime_core::MANAGED_STATE_COMMITMENT_KEY_V1,
                )?
                .ok_or_else(|| {
                    BackendError::Rpc(format!(
                        "external Service {service_id} has no finalized managed-state commitment"
                    ))
                })?;
            let root = service_runtime_core::ManagedStateCommitmentV1::decode(&commitment)
                .map_err(BackendError::Wire)?
                .root;
            let access_plan = StateAccessPlanV1::from_keys(keys).map_err(BackendError::Wire)?;
            let managed_state = state
                .provider
                .build_witness(record.service_key, root, &access_plan)
                .map_err(BackendError::Provider)?;
            Ok(service_runtime_core::ExternalStateWitnessV1 {
                service_id: *service_id,
                managed_state,
            })
        })
        .collect()
}

impl<T> MiniJamNetworkGateway<T> {
    pub fn new(transport: T, node_rpc: impl Into<String>, formal_rpc: impl Into<String>) -> Self {
        Self {
            transport,
            node_rpc: node_rpc.into(),
            formal_rpc: formal_rpc.into(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl<T: JsonRpcTransport + Send + Sync> BackendNetwork for MiniJamNetworkGateway<T> {
    fn genesis_hash(&self) -> Result<StateRoot, BackendError> {
        let value = self
            .transport
            .call(
                &self.node_rpc,
                "chain_getBlockHash",
                json!([0]),
                self.timeout,
                false,
            )
            .map_err(|error| BackendError::Rpc(error.message))?;
        parse_hash(
            value
                .as_str()
                .ok_or_else(|| BackendError::Rpc("genesis RPC returned no hash".into()))?,
        )
    }

    fn finalized_context(&self) -> Result<FinalizedContextV1, BackendError> {
        let value = self
            .transport
            .call(
                &self.node_rpc,
                "minijam_getFinalizedContext",
                json!([]),
                self.timeout,
                false,
            )
            .map_err(|error| BackendError::Rpc(error.message))?;
        Ok(FinalizedContextV1 {
            block_hash: parse_hash(
                value
                    .get("blockHash")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BackendError::Rpc("finalized context lacks blockHash".into()))?,
            )?,
            block_number: value
                .get("blockNumber")
                .and_then(Value::as_u64)
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(|| BackendError::Rpc("finalized context lacks blockNumber".into()))?,
            state_root: parse_hash(
                value
                    .get("stateRoot")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BackendError::Rpc("finalized context lacks stateRoot".into()))?,
            )?,
            slot: value
                .get("slot")
                .and_then(Value::as_u64)
                .and_then(|slot| u32::try_from(slot).ok())
                .ok_or_else(|| BackendError::Rpc("finalized context lacks slot".into()))?,
        })
    }

    fn service_info(
        &self,
        context: &FinalizedContextV1,
        service_id: u32,
    ) -> Result<Option<ChainServiceInfoV1>, BackendError> {
        let value = self
            .transport
            .call(
                &self.node_rpc,
                "minijam_getServiceInfoAt",
                json!([hash_hex(&context.block_hash), service_id]),
                self.timeout,
                false,
            )
            .map_err(|error| BackendError::Rpc(error.message))?;
        let Some(encoded) = value.as_str() else {
            return Ok(None);
        };
        let bytes = decode_hex(encoded)?;
        if bytes.len() < 33 {
            return Err(BackendError::Rpc("invalid SCALE ServiceInfo".into()));
        }
        let mut code_hash = [0u8; 32];
        code_hash.copy_from_slice(&bytes[1..33]);
        Ok(Some(ChainServiceInfoV1 {
            service_id,
            code_hash,
        }))
    }

    fn service_storage_at(
        &self,
        context: &FinalizedContextV1,
        service_id: u32,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let value = self
            .transport
            .call(
                &self.node_rpc,
                "minijam_getServiceStorageAt",
                json!([hash_hex(&context.block_hash), service_id, hash_hex(key)]),
                self.timeout,
                false,
            )
            .map_err(|error| BackendError::Rpc(error.message))?;
        value.as_str().map(decode_hex).transpose()
    }

    fn service_code(
        &self,
        context: &FinalizedContextV1,
        service_id: u32,
        code_hash: StateRoot,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let value = self
            .transport
            .call(
                &self.node_rpc,
                "minijam_getServicePreimageAt",
                json!([
                    hash_hex(&context.block_hash),
                    service_id,
                    hash_hex(&code_hash)
                ]),
                self.timeout,
                false,
            )
            .map_err(|error| BackendError::Rpc(error.message))?;
        value.as_str().map(decode_hex).transpose()
    }

    fn submit_work(&self, params: Value) -> Result<Value, BackendError> {
        self.transport
            .call(
                &self.formal_rpc,
                "minijam_submitWorkV1",
                params,
                self.timeout,
                true,
            )
            .map_err(|error| BackendError::Rpc(error.message))
    }

    fn work_status(&self, params: Value) -> Result<Value, BackendError> {
        self.transport
            .call(
                &self.formal_rpc,
                "minijam_getWorkStatusV1",
                params,
                self.timeout,
                false,
            )
            .map_err(|error| BackendError::Rpc(error.message))
    }
}

impl<T: JsonRpcTransport + Send + Sync> BackendWorkGateway for MiniJamNetworkGateway<T> {
    fn submit_work(&self, params: Value) -> Result<Value, BackendError> {
        <Self as BackendNetwork>::submit_work(self, params)
    }

    fn work_status(&self, params: Value) -> Result<Value, BackendError> {
        <Self as BackendNetwork>::work_status(self, params)
    }
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
    pvm_loader: Option<Arc<PvmArtifactLoader>>,
    artifact_store: Option<Arc<dyn ArtifactStore>>,
    persistence: Option<Arc<DiskBackendStore>>,
    network: Option<Arc<dyn BackendNetwork>>,
    dynamic_pvm_services: bool,
    admin_token: Option<String>,
}

impl BackendRpcHandler {
    pub fn new(state: BackendState, work: Arc<dyn BackendWorkGateway>) -> Self {
        Self {
            state: std::sync::Mutex::new(state),
            work,
            registration_validator: None,
            artifact_loader: None,
            pvm_loader: None,
            artifact_store: None,
            persistence: None,
            network: None,
            dynamic_pvm_services: false,
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

    pub fn with_pvm_artifact_loader(mut self, loader: Arc<PvmArtifactLoader>) -> Self {
        self.artifact_loader = Some(loader.clone());
        self.pvm_loader = Some(loader);
        self.dynamic_pvm_services = true;
        self
    }

    pub fn with_artifact_store(mut self, store: Arc<dyn ArtifactStore>) -> Self {
        self.artifact_store = Some(store);
        self
    }

    pub fn with_persistence(mut self, store: Arc<DiskBackendStore>) -> Self {
        self.persistence = Some(store);
        self
    }

    pub fn with_network(mut self, network: Arc<dyn BackendNetwork>) -> Self {
        self.network = Some(network);
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
            "chain_getBlockHash" => self.chain_get_block_hash(params),
            "minijam_getFinalizedContext" => self.finalized_context(),
            "minijam_getServiceStorageAt" => self.service_storage_at(params),
            "minijam_getServiceInfoAt" => self.service_info_at(params),
            "jamscript_getCapabilitiesV1" => Ok(serde_json::to_value(self.capabilities())
                .map_err(|error| BackendError::Rpc(error.to_string()))?),
            "jamscript_listServicesV1" => self.list_services(),
            "jamscript_registerServiceV1" => self.register_service(params),
            "jamscript_getServiceV1" => self.get_service(params),
            "jamscript_putArtifactV1" => self.put_artifact(params),
            "minijam_submitWorkV1" => self.submit_work(params),
            "minijam_getWorkStatusV1" => self.work_status(params),
            "minijam_getManagedStateV1" => self.get_managed_state(params),
            "jamscript_getPredictionV1" => self.get_prediction(params),
            _ => Err(BackendError::Rpc(format!("method not found: {method}"))),
        }
    }

    fn capabilities(&self) -> CapabilitiesJson {
        CapabilitiesJson::from(CapabilitiesV1 {
            dynamic_pvm_services: self.dynamic_pvm_services,
            ..CapabilitiesV1::default()
        })
    }

    fn chain_get_block_hash(&self, params: Value) -> Result<Value, BackendError> {
        let number = params
            .as_array()
            .and_then(|values| values.first())
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if number != 0 {
            return Err(BackendError::Rpc(
                "only genesis hash is exposed by the backend".into(),
            ));
        }
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("network gateway is not configured".into()))?;
        Ok(Value::String(hash_hex(&network.genesis_hash()?)))
    }

    fn finalized_context(&self) -> Result<Value, BackendError> {
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("network gateway is not configured".into()))?;
        let context = network.finalized_context()?;
        Ok(json!({
            "blockHash": hash_hex(&context.block_hash),
            "blockNumber": context.block_number,
            "stateRoot": hash_hex(&context.state_root),
            "slot": context.slot,
        }))
    }

    fn service_storage_at(&self, params: Value) -> Result<Value, BackendError> {
        let values = params
            .as_array()
            .ok_or_else(|| BackendError::Rpc("service storage params must be an array".into()))?;
        let block_hash = values
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| BackendError::Rpc("block hash is required".into()))?;
        let service_id = values
            .get(1)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| BackendError::Rpc("serviceId must be a u32".into()))?;
        let key = decode_hex(
            values
                .get(2)
                .and_then(Value::as_str)
                .ok_or_else(|| BackendError::Rpc("storage key is required".into()))?,
        )?;
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("network gateway is not configured".into()))?;
        let context = network.finalized_context()?;
        if parse_hash(block_hash)? != context.block_hash {
            return Err(BackendError::Rpc(
                "historical block is not finalized".into(),
            ));
        }
        Ok(network
            .service_storage_at(&context, service_id, &key)?
            .map(|bytes| Value::String(hash_hex(&bytes)))
            .unwrap_or(Value::Null))
    }

    fn service_info_at(&self, params: Value) -> Result<Value, BackendError> {
        let values = params
            .as_array()
            .ok_or_else(|| BackendError::Rpc("service info params must be an array".into()))?;
        let service_id = values
            .get(1)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| BackendError::Rpc("serviceId must be a u32".into()))?;
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("network gateway is not configured".into()))?;
        let context = network.finalized_context()?;
        Ok(network
            .service_info(&context, service_id)?
            .map(|info| Value::String(hash_hex(&info.code_hash)))
            .unwrap_or(Value::Null))
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
        if object.get("serviceKey").is_none() || object.get("codeHash").is_none() {
            return self.discover_service(params);
        }
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
        if let Some(store) = &self.persistence {
            store.persist_registry(&state.registry)?;
        }
        service_json(&record)
    }

    fn discover_service(&self, params: Value) -> Result<Value, BackendError> {
        let service_id = required_u32(&params, "serviceId")?;
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("network gateway is not configured".into()))?;
        let context = network.finalized_context()?;
        let chain = network
            .service_info(&context, service_id)?
            .ok_or(BackendError::UnknownService)?;
        let expected_digest = params
            .get("artifactDigest")
            .or_else(|| params.get("plannerArtifactDigest"))
            .and_then(Value::as_str)
            .map(parse_hash)
            .transpose()?;
        let store = self
            .artifact_store
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("artifact store is not configured".into()))?;
        let digest = if let Some(digest) = expected_digest {
            if !store.contains(digest)? {
                let bytes = network
                    .service_code(&context, service_id, chain.code_hash)?
                    .ok_or(BackendError::ArtifactNotFound)?;
                store.put_verified(digest, &bytes)?;
            }
            digest
        } else {
            let bytes = network
                .service_code(&context, service_id, chain.code_hash)?
                .ok_or(BackendError::ArtifactNotFound)?;
            let digest = service_runtime_core::blake2_256(&bytes);
            store.put_verified(digest, &bytes)?;
            digest
        };
        let artifact = ApplicationArtifactRef {
            digest,
            format: ArtifactFormat::JamScriptPvmV1,
        };
        let loader = self
            .artifact_loader
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("dynamic PVM loader is not configured".into()))?;
        if let Some(canonical_hash) = loader
            .canonical_code_hash(&artifact)
            .map_err(BackendError::Planner)?
        {
            if canonical_hash != chain.code_hash {
                return Err(BackendError::CodeHashMismatch);
            }
        }
        let metadata = loader
            .metadata(&artifact)
            .map_err(BackendError::Planner)?
            .ok_or(BackendError::InvalidMetadata)?;
        if let Some(expected) = params.get("serviceKey").and_then(Value::as_str) {
            if metadata.service_key != ServiceKeyV1::new(parse_hash(expected)?) {
                return Err(BackendError::ServiceKeyMismatch);
            }
        }
        let record = ServiceRecord {
            service_id,
            service_key: metadata.service_key,
            code_hash: chain.code_hash,
            abi_version: metadata.abi_version,
            application_artifact: artifact,
            deployment: DeploymentMetadata::default(),
        };
        let planner = loader
            .load(&record.application_artifact)
            .map_err(BackendError::Planner)?;
        let allow_upgrade = params
            .get("allowUpgrade")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        if let Ok(existing) = state.registry.get(service_id) {
            if existing.service_key != record.service_key || !allow_upgrade {
                return Err(BackendError::Rpc(
                    "Service is already registered; use an explicit validated upgrade".into(),
                ));
            }
        }
        state.register(record.clone())?;
        state.register_planner(service_id, planner)?;
        if let Some(store) = &self.persistence {
            store.persist_registry(&state.registry)?;
        }
        service_json(&record)
    }

    fn get_service(&self, params: Value) -> Result<Value, BackendError> {
        let service_id = required_u32(&params, "serviceId")?;
        let state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        let record = state.registry.get(service_id)?;
        service_json(record)
    }

    fn readiness(&self) -> Result<(), BackendError> {
        let state = self
            .state
            .lock()
            .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
        if let Some(store) = &self.artifact_store {
            if let Some(root) = store.root() {
                if !root.is_dir() {
                    return Err(BackendError::ArtifactStore(
                        "artifact store is unavailable".into(),
                    ));
                }
            }
        }
        drop(state);
        if let Some(network) = &self.network {
            network.finalized_context()?;
        }
        Ok(())
    }

    fn put_artifact(&self, params: Value) -> Result<Value, BackendError> {
        ArtifactFormat::parse(required_str(&params, "format")?)?;
        let digest = parse_hash(required_str(&params, "digest")?)?;
        let encoded = required_str(&params, "bytesBase64")?;
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| BackendError::Rpc(format!("invalid bytesBase64: {error}")))?;
        let store = self
            .artifact_store
            .as_ref()
            .ok_or_else(|| BackendError::Rpc("artifact store is not configured".into()))?;
        let size = store.put_verified(digest, &bytes)?;
        Ok(json!({
            "format": ArtifactFormat::WIRE_NAME,
            "digest": hash_hex(&digest),
            "size": size,
            "idempotent": true,
        }))
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
        if let (Some(network), Some(loader)) = (&self.network, &self.pvm_loader) {
            let needs_discovery = self
                .state
                .lock()
                .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?
                .registry
                .get(service_id)
                .is_err();
            if needs_discovery {
                self.discover_service(json!({
                    "serviceId": service_id,
                    "artifactDigest": params.get("artifactDigest"),
                }))?;
            }
            let mut state = self
                .state
                .lock()
                .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
            return BackendEngine::new(Arc::clone(network), Arc::clone(loader))
                .submit(&mut state, params);
        }
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
        if let (Some(network), Some(loader)) = (&self.network, &self.pvm_loader) {
            let mut state = self
                .state
                .lock()
                .map_err(|_| BackendError::Rpc("state lock poisoned".into()))?;
            let key = params
                .get("packageHash")
                .and_then(Value::as_str)
                .map(parse_hash)
                .transpose()?;
            let service_id = params
                .get("serviceId")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            let prediction = match (service_id, key) {
                (Some(service_id), Some(package_hash)) => state
                    .prediction(service_id, package_hash)
                    .cloned()
                    .map(|output| {
                        (
                            WorkKey {
                                service_id,
                                package_hash,
                            },
                            output,
                        )
                    }),
                _ => None,
            };
            let was_finalized = prediction
                .as_ref()
                .is_some_and(|(key, _)| state.is_finalized(*key));
            let result = BackendEngine::new(Arc::clone(network), Arc::clone(loader))
                .status(&mut state, params)?;
            if let Some((key, output)) = prediction {
                if !was_finalized && state.is_finalized(key) {
                    if let Some(store) = &self.persistence {
                        let service_key = state.registry.resolve_key(key.service_id)?;
                        store.persist_registry(&state.registry)?;
                        state.append_recovery(
                            store.recovery_path(),
                            &RecoveryEnvelopeV1 {
                                service_id: key.service_id,
                                service_key,
                                output,
                            },
                        )?;
                    }
                }
            }
            return Ok(result);
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
    dynamic_pvm_services: bool,
}

impl Default for CapabilitiesJson {
    fn default() -> Self {
        let capabilities = CapabilitiesV1::default();
        Self {
            protocol_version: capabilities.protocol_version,
            managed_state_version: capabilities.managed_state_version,
            multi_service: capabilities.multi_service,
            external_state_witness: capabilities.external_state_witness,
            dynamic_pvm_services: capabilities.dynamic_pvm_services,
        }
    }
}

impl From<CapabilitiesV1> for CapabilitiesJson {
    fn from(capabilities: CapabilitiesV1) -> Self {
        Self {
            protocol_version: capabilities.protocol_version,
            managed_state_version: capabilities.managed_state_version,
            multi_service: capabilities.multi_service,
            external_state_witness: capabilities.external_state_witness,
            dynamic_pvm_services: capabilities.dynamic_pvm_services,
        }
    }
}

/// Minimal single-endpoint HTTP daemon. A production deployment can put TLS
/// and authentication in front of this listener; the routing/state contract
/// remains the same.
pub struct BackendDaemon {
    bind: String,
    handler: Arc<BackendRpcHandler>,
    active_connections: Arc<AtomicUsize>,
}

impl BackendDaemon {
    pub fn new(bind: impl Into<String>, handler: Arc<BackendRpcHandler>) -> Self {
        Self {
            bind: bind.into(),
            handler,
            active_connections: Arc::new(AtomicUsize::new(0)),
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
                    if self
                        .active_connections
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                            (active < 64).then_some(active + 1)
                        })
                        .is_err()
                    {
                        continue;
                    }
                    let handler = Arc::clone(&self.handler);
                    let active_connections = Arc::clone(&self.active_connections);
                    std::thread::spawn(move || {
                        if let Err(error) = serve_connection(stream, &handler) {
                            eprintln!("backend HTTP connection error: {error}");
                        }
                        active_connections.fetch_sub(1, Ordering::AcqRel);
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
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| BackendError::Rpc(error.to_string()))?;
    let (method, path, body) = read_http_request(&mut stream)?;
    let (status, response) = if method == "GET" && (path == "/healthz" || path == "/health/ready") {
        ("200 OK", br#"{"status":"ready"}"#.to_vec())
    } else if method == "GET" && path == "/readinessz" {
        match handler.readiness() {
            Ok(()) => ("200 OK", br#"{"status":"ready"}"#.to_vec()),
            Err(error) => (
                "503 Service Unavailable",
                json!({"status":"not_ready","error":rpc_error(&error)})
                    .to_string()
                    .into_bytes(),
            ),
        }
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

// This includes base64 JSON uploads. It is deliberately larger than the
// binary artifact bound so a valid maximum-sized PVM can still be uploaded.
const MAX_HTTP_BYTES: usize = 32 * 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;

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
            if header_end > MAX_HTTP_HEADER_BYTES {
                return Err(BackendError::Rpc("HTTP headers too large".into()));
            }
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
    let mut content_length = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(BackendError::Rpc("malformed HTTP header".into()));
        };
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| BackendError::Rpc("invalid Content-Length".into()))?;
            if content_length.replace(parsed).is_some() {
                return Err(BackendError::Rpc("duplicate Content-Length".into()));
            }
        }
    }
    let length = content_length.unwrap_or(0);
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
        "artifact": {
            "format": ArtifactFormat::WIRE_NAME,
            "digest": hash_hex(&record.application_artifact.digest),
        },
    }))
}

fn parse_service_record(params: &Value) -> Result<ServiceRecord, BackendError> {
    let artifact = params
        .get("artifact")
        .or_else(|| params.get("plannerArtifact"));
    let digest = if let Some(value) = artifact
        .and_then(Value::as_object)
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
    let format = artifact
        .and_then(Value::as_object)
        .and_then(|object| object.get("format"))
        .and_then(Value::as_str)
        .map(ArtifactFormat::parse)
        .transpose()?
        .unwrap_or(ArtifactFormat::JamScriptPvmV1);
    Ok(ServiceRecord {
        service_id: required_u32(params, "serviceId")?,
        service_key: ServiceKeyV1::new(parse_hash(required_str(params, "serviceKey")?)?),
        code_hash: parse_hash(required_str(params, "codeHash")?)?,
        abi_version: required_u32(params, "abiVersion")?,
        application_artifact: ApplicationArtifactRef { digest, format },
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

fn decode_hex(value: &str) -> Result<Vec<u8>, BackendError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if !value.len().is_multiple_of(2) {
        return Err(BackendError::Rpc("invalid hexadecimal byte string".into()));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
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
        BackendError::StaleContext => (-32041, "finalized context is stale".into()),
        BackendError::PredictionStale => (
            -32042,
            "canonical managed-state root disagrees with the predicted transition".into(),
        ),
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
    finalized: std::collections::BTreeSet<WorkKey>,
}

impl BackendState {
    pub fn with_registry(registry: ServiceRegistry) -> Self {
        Self {
            registry,
            ..Self::default()
        }
    }

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
        self.finalized.remove(&key);
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

    pub fn is_finalized(&self, key: WorkKey) -> bool {
        self.finalized.contains(&key)
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
        self.finalized.insert(key);
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
                format: ArtifactFormat::JamScriptPvmV1,
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
            let key = if artifact.digest == [3; 32] {
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
    fn disk_artifacts_are_content_addressed_and_fail_closed_on_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let store = DiskArtifactStore::new(directory.path()).unwrap();
        let bytes = b"linked-pvm-artifact";
        let digest = service_runtime_core::blake2_256(bytes);
        assert_eq!(store.put_verified(digest, bytes).unwrap(), bytes.len());
        assert_eq!(store.put_verified(digest, bytes).unwrap(), bytes.len());
        assert_eq!(store.get_verified(digest).unwrap(), bytes);
        assert!(store.contains(digest).unwrap());
        assert_eq!(
            store.put_verified([9; 32], bytes),
            Err(BackendError::ArtifactDigestMismatch)
        );

        fs::write(store.path(&digest), b"corrupted").unwrap();
        assert_eq!(
            store.get_verified(digest),
            Err(BackendError::ArtifactCorrupt)
        );
        assert_eq!(store.contains(digest), Err(BackendError::ArtifactCorrupt));
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
                "format": ArtifactFormat::WIRE_NAME
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
                    "format": ArtifactFormat::WIRE_NAME,
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
