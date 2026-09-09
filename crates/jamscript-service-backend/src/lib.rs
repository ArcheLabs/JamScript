use service_runtime_core::{
    RuntimeRefineOutputV1, ServiceKeyV1, StateQueryResponseV1, StateRoot,
};
use service_runtime_host::{
    FullStateProvider, MaterializedServiceStateProvider, ServiceStateProvider,
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

const RECOVERY_ENVELOPE_MAGIC: &[u8; 4] = b"JSR1";
const MAX_RECOVERY_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceRecord {
    pub service_id: u32,
    pub service_key: ServiceKeyV1,
    pub code_hash: [u8; 32],
}

#[derive(Clone, Debug, Default)]
pub struct ServiceRegistry {
    by_id: BTreeMap<u32, ServiceRecord>,
    by_key: BTreeMap<ServiceKeyV1, u32>,
}

impl ServiceRegistry {
    pub fn register(&mut self, record: ServiceRecord) -> Result<(), BackendError> {
        if let Some(existing_id) = self.by_key.get(&record.service_key).copied() {
            if existing_id != record.service_id {
                return Err(BackendError::ServiceKeyAlreadyRegistered {
                    service_id: existing_id,
                });
            }
        }
        if let Some(existing) = self.by_id.get(&record.service_id) {
            if existing.service_key != record.service_key {
                return Err(BackendError::ServiceIdAlreadyRegistered {
                    service_id: record.service_id,
                });
            }
        }

        self.by_key.insert(record.service_key, record.service_id);
        self.by_id.insert(record.service_id, record);
        Ok(())
    }

    pub fn get(&self, service_id: u32) -> Option<&ServiceRecord> {
        self.by_id.get(&service_id)
    }

    pub fn service_id_for_key(&self, service_key: ServiceKeyV1) -> Option<u32> {
        self.by_key.get(&service_key).copied()
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

pub type PackageHash = [u8; 32];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkKey {
    pub service_id: u32,
    pub package_hash: PackageHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaterializeDecision {
    NotTracked,
    Waiting {
        parent_root: StateRoot,
        predicted_root: StateRoot,
    },
    Applied {
        root: StateRoot,
    },
    Diverged {
        parent_root: StateRoot,
        predicted_root: StateRoot,
        finalized_root: StateRoot,
    },
}

#[derive(Clone, Debug)]
pub struct BackendState {
    registry: ServiceRegistry,
    provider: FullStateProvider,
    pending: BTreeMap<WorkKey, RuntimeRefineOutputV1>,
    predictions: BTreeMap<WorkKey, RuntimeRefineOutputV1>,
}

impl Default for BackendState {
    fn default() -> Self {
        Self {
            registry: ServiceRegistry::default(),
            provider: FullStateProvider::default(),
            pending: BTreeMap::new(),
            predictions: BTreeMap::new(),
        }
    }
}

impl BackendState {
    pub fn registry(&self) -> &ServiceRegistry {
        &self.registry
    }

    pub fn provider(&self) -> &FullStateProvider {
        &self.provider
    }

    pub fn register_service(&mut self, record: ServiceRecord) -> Result<(), BackendError> {
        self.registry.register(record)
    }

    pub fn service(&self, service_id: u32) -> Result<ServiceRecord, BackendError> {
        self.registry
            .get(service_id)
            .copied()
            .ok_or(BackendError::UnknownService { service_id })
    }

    pub fn track_prediction(
        &mut self,
        service_id: u32,
        package_hash: PackageHash,
        output: RuntimeRefineOutputV1,
    ) -> Result<(), BackendError> {
        let _ = self.service(service_id)?;
        let key = WorkKey {
            service_id,
            package_hash,
        };
        self.pending.insert(key, output.clone());
        self.predictions.insert(key, output);
        Ok(())
    }

    pub fn prediction(
        &self,
        service_id: u32,
        package_hash: PackageHash,
    ) -> Option<&RuntimeRefineOutputV1> {
        self.predictions.get(&WorkKey {
            service_id,
            package_hash,
        })
    }

    pub fn materialize_if_finalized(
        &mut self,
        service_id: u32,
        package_hash: PackageHash,
        finalized_root: StateRoot,
    ) -> Result<MaterializeDecision, BackendError> {
        let record = self.service(service_id)?;
        let key = WorkKey {
            service_id,
            package_hash,
        };
        let Some(output) = self.pending.get(&key).cloned() else {
            return Ok(MaterializeDecision::NotTracked);
        };

        if finalized_root == output.parent_root {
            return Ok(MaterializeDecision::Waiting {
                parent_root: output.parent_root,
                predicted_root: output.new_root,
            });
        }

        if finalized_root != output.new_root {
            self.pending.remove(&key);
            return Ok(MaterializeDecision::Diverged {
                parent_root: output.parent_root,
                predicted_root: output.new_root,
                finalized_root,
            });
        }

        self.provider
            .apply_recovery(record.service_key, &output)
            .map_err(|error| BackendError::Provider(format!("{error:?}")))?;
        self.pending.remove(&key);
        Ok(MaterializeDecision::Applied {
            root: output.new_root,
        })
    }

    pub fn apply_finalized_recovery(
        &mut self,
        service_id: u32,
        service_key: ServiceKeyV1,
        output: &RuntimeRefineOutputV1,
    ) -> Result<(), BackendError> {
        let registered = self.service(service_id)?;
        if registered.service_key != service_key {
            return Err(BackendError::RecoveryServiceMismatch { service_id });
        }
        self.provider
            .apply_recovery(service_key, output)
            .map_err(|error| BackendError::Provider(format!("{error:?}")))
    }

    pub fn query(
        &self,
        service_id: u32,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, BackendError> {
        let record = self.service(service_id)?;
        self.provider
            .get(record.service_key, root, key)
            .map_err(|error| BackendError::Provider(format!("{error:?}")))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryEnvelopeV1 {
    pub service_id: u32,
    pub service_key: ServiceKeyV1,
    pub output: RuntimeRefineOutputV1,
}

impl RecoveryEnvelopeV1 {
    pub fn encode(&self) -> Result<Vec<u8>, BackendError> {
        let output = self
            .output
            .encode()
            .map_err(|_| BackendError::InvalidRecoveryEnvelope)?;
        let output_len =
            u32::try_from(output.len()).map_err(|_| BackendError::RecoveryEnvelopeTooLarge)?;
        let capacity = 4usize
            .checked_add(4)
            .and_then(|value| value.checked_add(32))
            .and_then(|value| value.checked_add(4))
            .and_then(|value| value.checked_add(output.len()))
            .ok_or(BackendError::RecoveryEnvelopeTooLarge)?;
        if capacity > MAX_RECOVERY_ENVELOPE_BYTES {
            return Err(BackendError::RecoveryEnvelopeTooLarge);
        }
        let mut encoded = Vec::with_capacity(capacity);
        encoded.extend_from_slice(RECOVERY_ENVELOPE_MAGIC);
        encoded.extend_from_slice(&self.service_id.to_le_bytes());
        encoded.extend_from_slice(self.service_key.as_bytes());
        encoded.extend_from_slice(&output_len.to_le_bytes());
        encoded.extend_from_slice(&output);
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, BackendError> {
        if bytes.len() < 44 || bytes.len() > MAX_RECOVERY_ENVELOPE_BYTES {
            return Err(BackendError::InvalidRecoveryEnvelope);
        }
        if bytes.get(..4) != Some(RECOVERY_ENVELOPE_MAGIC.as_slice()) {
            return Err(BackendError::InvalidRecoveryEnvelope);
        }
        let service_id = u32::from_le_bytes(
            bytes[4..8]
                .try_into()
                .map_err(|_| BackendError::InvalidRecoveryEnvelope)?,
        );
        let service_key = ServiceKeyV1::new(
            bytes[8..40]
                .try_into()
                .map_err(|_| BackendError::InvalidRecoveryEnvelope)?,
        );
        let output_len = u32::from_le_bytes(
            bytes[40..44]
                .try_into()
                .map_err(|_| BackendError::InvalidRecoveryEnvelope)?,
        ) as usize;
        if bytes.len() != 44usize.saturating_add(output_len) {
            return Err(BackendError::InvalidRecoveryEnvelope);
        }
        let output = RuntimeRefineOutputV1::decode(&bytes[44..])
            .map_err(|_| BackendError::InvalidRecoveryEnvelope)?;
        Ok(Self {
            service_id,
            service_key,
            output,
        })
    }
}

pub fn append_recovery(path: &Path, envelope: &RecoveryEnvelopeV1) -> Result<(), BackendError> {
    let encoded = envelope.encode()?;
    let length =
        u32::try_from(encoded.len()).map_err(|_| BackendError::RecoveryEnvelopeTooLarge)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(BackendError::Io)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(BackendError::Io)?;
    file.write_all(&length.to_le_bytes())
        .and_then(|_| file.write_all(&encoded))
        .and_then(|_| file.sync_data())
        .map_err(BackendError::Io)
}

pub fn replay_recoveries(path: &Path, state: &mut BackendState) -> Result<usize, BackendError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(BackendError::Io(error)),
    };
    let mut offset = 0usize;
    let mut count = 0usize;
    while offset < bytes.len() {
        let length_end = offset
            .checked_add(4)
            .ok_or(BackendError::InvalidRecoveryLog)?;
        let length_bytes = bytes
            .get(offset..length_end)
            .ok_or(BackendError::InvalidRecoveryLog)?;
        let length = u32::from_le_bytes(
            length_bytes
                .try_into()
                .map_err(|_| BackendError::InvalidRecoveryLog)?,
        ) as usize;
        if length == 0 || length > MAX_RECOVERY_ENVELOPE_BYTES {
            return Err(BackendError::InvalidRecoveryLog);
        }
        offset = length_end;
        let entry_end = offset
            .checked_add(length)
            .ok_or(BackendError::InvalidRecoveryLog)?;
        let entry = bytes
            .get(offset..entry_end)
            .ok_or(BackendError::InvalidRecoveryLog)?;
        offset = entry_end;
        let envelope = RecoveryEnvelopeV1::decode(entry)?;
        state.apply_finalized_recovery(
            envelope.service_id,
            envelope.service_key,
            &envelope.output,
        )?;
        count = count.saturating_add(1);
    }
    Ok(count)
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("unknown Service {service_id}")]
    UnknownService { service_id: u32 },
    #[error("ServiceId {service_id} is already registered to a different ServiceKey")]
    ServiceIdAlreadyRegistered { service_id: u32 },
    #[error("ServiceKey is already registered to ServiceId {service_id}")]
    ServiceKeyAlreadyRegistered { service_id: u32 },
    #[error("recovery ServiceKey does not match registered ServiceId {service_id}")]
    RecoveryServiceMismatch { service_id: u32 },
    #[error("managed-state provider error: {0}")]
    Provider(String),
    #[error("invalid recovery envelope")]
    InvalidRecoveryEnvelope,
    #[error("recovery envelope exceeds the backend limit")]
    RecoveryEnvelopeTooLarge,
    #[error("invalid recovery log")]
    InvalidRecoveryLog,
    #[error("backend I/O error: {0}")]
    Io(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_runtime_core::{
        StateChangeV1, StateDiffV1, StateRecoveryV1, MANAGED_STATE_PROTOCOL_VERSION,
        RECOVERY_FORMAT_VERSION,
    };
    use service_runtime_state::FullState;

    fn record(service_id: u32, byte: u8) -> ServiceRecord {
        ServiceRecord {
            service_id,
            service_key: ServiceKeyV1::new([byte; 32]),
            code_hash: [byte.wrapping_add(1); 32],
        }
    }

    fn transition(key: &[u8], value: u8) -> RuntimeRefineOutputV1 {
        let diff = StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: key.to_vec(),
                value: Some(vec![value]),
            }],
        };
        let recovery = StateRecoveryV1 {
            version: RECOVERY_FORMAT_VERSION,
            diff: diff.clone(),
        };
        let parent = FullState::empty();
        let next = parent.apply_diff(&diff).expect("valid transition");
        RuntimeRefineOutputV1 {
            version: MANAGED_STATE_PROTOCOL_VERSION,
            parent_root: parent.root(),
            new_root: next.root(),
            transition_valid_until: None,
            receipts: Vec::new(),
            recovery_commitment: recovery.commitment().expect("valid recovery"),
            recovery_payload: recovery.encode().expect("valid recovery"),
        }
    }

    #[test]
    fn registry_is_multi_service_and_allows_code_hash_updates() {
        let mut registry = ServiceRegistry::default();
        registry.register(record(10, 1)).unwrap();
        registry.register(record(11, 2)).unwrap();
        let mut upgraded = record(10, 1);
        upgraded.code_hash = [9; 32];
        registry.register(upgraded).unwrap();
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get(10), Some(&upgraded));
        assert_eq!(registry.service_id_for_key(ServiceKeyV1::new([2; 32])), Some(11));
    }

    #[test]
    fn work_tracking_is_scoped_by_service_id() {
        let mut state = BackendState::default();
        state.register_service(record(10, 1)).unwrap();
        state.register_service(record(11, 2)).unwrap();
        let package_hash = [7; 32];
        let first = transition(b"a", 1);
        let second = transition(b"b", 2);
        state
            .track_prediction(10, package_hash, first.clone())
            .unwrap();
        state
            .track_prediction(11, package_hash, second.clone())
            .unwrap();
        assert_eq!(state.prediction(10, package_hash), Some(&first));
        assert_eq!(state.prediction(11, package_hash), Some(&second));
    }

    #[test]
    fn finalized_materialization_is_isolated_per_service() {
        let mut state = BackendState::default();
        state.register_service(record(10, 1)).unwrap();
        state.register_service(record(11, 2)).unwrap();
        let first = transition(b"a", 1);
        let second = transition(b"b", 2);
        state.track_prediction(10, [1; 32], first.clone()).unwrap();
        state.track_prediction(11, [2; 32], second.clone()).unwrap();

        assert!(matches!(
            state
                .materialize_if_finalized(10, [1; 32], first.new_root)
                .unwrap(),
            MaterializeDecision::Applied { .. }
        ));
        assert!(matches!(
            state
                .materialize_if_finalized(11, [2; 32], second.new_root)
                .unwrap(),
            MaterializeDecision::Applied { .. }
        ));

        assert_eq!(
            state.query(10, first.new_root, b"a").unwrap().value,
            Some(vec![1])
        );
        assert_eq!(
            state.query(11, second.new_root, b"b").unwrap().value,
            Some(vec![2])
        );
    }

    #[test]
    fn recovery_log_carries_service_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.log");
        let first_record = record(10, 1);
        let second_record = record(11, 2);
        let first = transition(b"a", 1);
        let second = transition(b"b", 2);
        append_recovery(
            &path,
            &RecoveryEnvelopeV1 {
                service_id: first_record.service_id,
                service_key: first_record.service_key,
                output: first.clone(),
            },
        )
        .unwrap();
        append_recovery(
            &path,
            &RecoveryEnvelopeV1 {
                service_id: second_record.service_id,
                service_key: second_record.service_key,
                output: second.clone(),
            },
        )
        .unwrap();

        let mut restored = BackendState::default();
        restored.register_service(first_record).unwrap();
        restored.register_service(second_record).unwrap();
        assert_eq!(replay_recoveries(&path, &mut restored).unwrap(), 2);
        assert_eq!(
            restored.query(10, first.new_root, b"a").unwrap().value,
            Some(vec![1])
        );
        assert_eq!(
            restored.query(11, second.new_root, b"b").unwrap().value,
            Some(vec![2])
        );
    }
}
