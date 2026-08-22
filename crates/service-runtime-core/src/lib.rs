#![no_std]

extern crate alloc;

use alloc::vec::Vec;
pub use jamscript_crypto::blake2_256;

pub type StateRoot = [u8; 32];

pub const MANAGED_STATE_PROTOCOL_VERSION: u8 = 1;
pub const MANAGED_STATE_LAYOUT_VERSION: u8 = 1;
pub const RECOVERY_FORMAT_VERSION: u8 = 1;
pub const EMPTY_STATE_ROOT_V1: StateRoot = [
    0x03, 0x17, 0x0a, 0x2e, 0x75, 0x97, 0xb7, 0xb7, 0xe3, 0xd8, 0x4c, 0x05, 0x39, 0x1d, 0x13, 0x9a,
    0x62, 0xb1, 0x57, 0xe7, 0x87, 0x86, 0xd8, 0xc0, 0x82, 0xf2, 0x9d, 0xcf, 0x4c, 0x11, 0x13, 0x14,
];
pub const MANAGED_STATE_COMMITMENT_KEY_V1: &[u8] = b":jam-service-runtime:managed-state:v1";
pub const APPLICATION_KEY_CLASS_V1: u8 = 0x01;
pub const RUNTIME_KEY_CLASS_V1: u8 = 0x00;
pub const WALLET_AUTH_MODULE_V1: u8 = 0x01;
pub const MAX_RUNTIME_ACTIONS: usize = 1024;
pub const MAX_WITNESS_NODES: usize = 4096;
pub const MAX_WITNESS_NODE_BYTES: usize = 64 * 1024;
pub const MAX_WITNESS_BYTES: usize = 1024 * 1024;
pub const MAX_WITNESS_ENCODED_BYTES: usize =
    1 + 32 + 4 + (MAX_WITNESS_NODES * 4) + MAX_WITNESS_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedStateCommitmentV1 {
    pub protocol_version: u8,
    pub layout_version: u8,
    pub root: StateRoot,
}

impl ManagedStateCommitmentV1 {
    pub const EMPTY: Self = Self {
        protocol_version: MANAGED_STATE_PROTOCOL_VERSION,
        layout_version: MANAGED_STATE_LAYOUT_VERSION,
        root: EMPTY_STATE_ROOT_V1,
    };

    pub fn new(root: StateRoot) -> Self {
        Self {
            protocol_version: MANAGED_STATE_PROTOCOL_VERSION,
            layout_version: MANAGED_STATE_LAYOUT_VERSION,
            root,
        }
    }

    pub fn encode(&self) -> [u8; 34] {
        let mut encoded = [0; 34];
        encoded[0] = self.protocol_version;
        encoded[1] = self.layout_version;
        encoded[2..].copy_from_slice(&self.root);
        encoded
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != 34 {
            return Err(WireError::InvalidLength);
        }
        let mut root = [0; 32];
        root.copy_from_slice(&bytes[2..]);
        let value = Self {
            protocol_version: bytes[0],
            layout_version: bytes[1],
            root,
        };
        if value.protocol_version != MANAGED_STATE_PROTOCOL_VERSION
            || value.layout_version != MANAGED_STATE_LAYOUT_VERSION
        {
            return Err(WireError::UnsupportedVersion);
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireError {
    InvalidLength,
    UnexpectedEof,
    InvalidEncoding,
    UnsupportedVersion,
    LengthOverflow,
    DuplicateKey,
    UnsortedKeys,
    TooManyItems,
}

pub fn application_key_v1(
    namespace: &[u8],
    canonical_user_key: &[u8],
) -> Result<Vec<u8>, WireError> {
    let namespace_len = u16::try_from(namespace.len()).map_err(|_| WireError::LengthOverflow)?;
    let mut key = Vec::with_capacity(3 + namespace.len() + canonical_user_key.len());
    key.push(APPLICATION_KEY_CLASS_V1);
    key.extend_from_slice(&namespace_len.to_le_bytes());
    key.extend_from_slice(namespace);
    key.extend_from_slice(canonical_user_key);
    Ok(key)
}

pub fn runtime_internal_key_v1(module: u8, module_key: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + module_key.len());
    key.push(RUNTIME_KEY_CLASS_V1);
    key.push(module);
    key.extend_from_slice(module_key);
    key
}

pub fn wallet_nonce_key_v1(account: &[u8; 32]) -> Vec<u8> {
    runtime_internal_key_v1(WALLET_AUTH_MODULE_V1, account)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateChangeV1 {
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct StateDiffV1 {
    pub changes: Vec<StateChangeV1>,
}

impl StateDiffV1 {
    pub fn canonicalize(&mut self) -> Result<(), WireError> {
        self.changes.sort_by(|left, right| left.key.cmp(&right.key));
        for pair in self.changes.windows(2) {
            if pair[0].key == pair[1].key {
                return Err(WireError::DuplicateKey);
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut canonical = self.clone();
        canonical.canonicalize()?;
        let count =
            u32::try_from(canonical.changes.len()).map_err(|_| WireError::LengthOverflow)?;
        let mut writer = Writer::new();
        writer.u8(MANAGED_STATE_PROTOCOL_VERSION);
        writer.u32(count);
        for change in canonical.changes {
            writer.bytes_u32(&change.key)?;
            match change.value {
                Some(value) => {
                    writer.u8(1);
                    writer.bytes_u32(&value)?;
                }
                None => writer.u8(0),
            }
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        if reader.u8()? != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let count = reader.u32()? as usize;
        if count > MAX_RUNTIME_ACTIONS * 1024 {
            return Err(WireError::TooManyItems);
        }
        let mut changes = Vec::with_capacity(count);
        for _ in 0..count {
            let key = reader.bytes_u32()?;
            let value = match reader.u8()? {
                0 => None,
                1 => Some(reader.bytes_u32()?),
                _ => return Err(WireError::InvalidEncoding),
            };
            changes.push(StateChangeV1 { key, value });
        }
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        let diff = Self { changes };
        let mut sorted = diff.clone();
        sorted.canonicalize()?;
        if sorted != diff {
            return Err(WireError::UnsortedKeys);
        }
        Ok(diff)
    }

    pub fn hash(&self) -> Result<StateRoot, WireError> {
        Ok(blake2_256(&self.encode()?))
    }
}

pub fn state_delta_hash(delta: &StateDiffV1) -> Result<StateRoot, WireError> {
    delta.hash()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateTransitionV1 {
    pub version: u8,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub diff_hash: StateRoot,
}

impl StateTransitionV1 {
    pub fn encode(&self) -> [u8; 97] {
        let mut bytes = [0; 97];
        bytes[0] = self.version;
        bytes[1..33].copy_from_slice(&self.parent_root);
        bytes[33..65].copy_from_slice(&self.new_root);
        bytes[65..97].copy_from_slice(&self.diff_hash);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != 97 {
            return Err(WireError::InvalidLength);
        }
        if bytes[0] != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let mut parent_root = [0; 32];
        let mut new_root = [0; 32];
        let mut diff_hash = [0; 32];
        parent_root.copy_from_slice(&bytes[1..33]);
        new_root.copy_from_slice(&bytes[33..65]);
        diff_hash.copy_from_slice(&bytes[65..]);
        Ok(Self {
            version: bytes[0],
            parent_root,
            new_root,
            diff_hash,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedStateWitnessV1 {
    pub version: u8,
    pub parent_root: StateRoot,
    pub storage_proof: Vec<Vec<u8>>,
}

impl ManagedStateWitnessV1 {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.storage_proof.len() > MAX_WITNESS_NODES {
            return Err(WireError::TooManyItems);
        }
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.raw(&self.parent_root);
        let count =
            u32::try_from(self.storage_proof.len()).map_err(|_| WireError::LengthOverflow)?;
        writer.u32(count);
        let mut total_bytes = 0usize;
        for node in &self.storage_proof {
            if node.len() > MAX_WITNESS_NODE_BYTES {
                return Err(WireError::TooManyItems);
            }
            total_bytes = total_bytes
                .checked_add(node.len())
                .ok_or(WireError::LengthOverflow)?;
            if total_bytes > MAX_WITNESS_BYTES {
                return Err(WireError::TooManyItems);
            }
            writer.bytes_u32(node)?;
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let parent_root = reader.array::<32>()?;
        let count = reader.u32()? as usize;
        if count > MAX_WITNESS_NODES {
            return Err(WireError::TooManyItems);
        }
        let mut storage_proof = Vec::with_capacity(count);
        let mut total_bytes = 0usize;
        for _ in 0..count {
            let remaining = MAX_WITNESS_BYTES
                .checked_sub(total_bytes)
                .ok_or(WireError::TooManyItems)?;
            let node = reader.bytes_limited(remaining.min(MAX_WITNESS_NODE_BYTES))?;
            total_bytes = total_bytes
                .checked_add(node.len())
                .ok_or(WireError::LengthOverflow)?;
            storage_proof.push(node);
        }
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self {
            version,
            parent_root,
            storage_proof,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRefineInputV1 {
    pub version: u8,
    pub managed_state: ManagedStateWitnessV1,
    pub actions: Vec<Vec<u8>>,
}

impl RuntimeRefineInputV1 {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.u8(self.version);
        let witness = self.managed_state.encode()?;
        writer.bytes_u32(&witness)?;
        let count = u32::try_from(self.actions.len()).map_err(|_| WireError::LengthOverflow)?;
        if self.actions.len() > MAX_RUNTIME_ACTIONS {
            return Err(WireError::TooManyItems);
        }
        writer.u32(count);
        for action in &self.actions {
            writer.bytes_u32(action)?;
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let witness =
            ManagedStateWitnessV1::decode(&reader.bytes_limited(MAX_WITNESS_ENCODED_BYTES)?)?;
        let count = reader.u32()? as usize;
        if count > MAX_RUNTIME_ACTIONS {
            return Err(WireError::TooManyItems);
        }
        let mut actions = Vec::with_capacity(count);
        for _ in 0..count {
            actions.push(reader.bytes_u32()?);
        }
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self {
            version,
            managed_state: witness,
            actions,
        })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionStatusV1 {
    Applied = 0,
    Failed = 1,
    Rejected = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionReceiptV1 {
    pub action_hash: StateRoot,
    pub status: ActionStatusV1,
    pub error_code: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRefineOutputV1 {
    pub version: u8,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub receipts: Vec<ActionReceiptV1>,
    pub recovery_commitment: Option<StateRoot>,
}

impl RuntimeRefineOutputV1 {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.raw(&self.parent_root);
        writer.raw(&self.new_root);
        let count = u32::try_from(self.receipts.len()).map_err(|_| WireError::LengthOverflow)?;
        writer.u32(count);
        for receipt in &self.receipts {
            writer.raw(&receipt.action_hash);
            writer.u8(receipt.status as u8);
            match receipt.error_code {
                Some(error) => {
                    writer.u8(1);
                    writer.u32(error);
                }
                None => writer.u8(0),
            }
        }
        match self.recovery_commitment {
            Some(root) => {
                writer.u8(1);
                writer.raw(&root);
            }
            None => writer.u8(0),
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let parent_root = reader.array::<32>()?;
        let new_root = reader.array::<32>()?;
        let count = reader.u32()? as usize;
        if count > MAX_RUNTIME_ACTIONS {
            return Err(WireError::TooManyItems);
        }
        let mut receipts = Vec::with_capacity(count);
        for _ in 0..count {
            let action_hash = reader.array::<32>()?;
            let status = match reader.u8()? {
                0 => ActionStatusV1::Applied,
                1 => ActionStatusV1::Failed,
                2 => ActionStatusV1::Rejected,
                _ => return Err(WireError::InvalidEncoding),
            };
            let error_code = match reader.u8()? {
                0 => None,
                1 => Some(reader.u32()?),
                _ => return Err(WireError::InvalidEncoding),
            };
            receipts.push(ActionReceiptV1 {
                action_hash,
                status,
                error_code,
            });
        }
        let recovery_commitment = match reader.u8()? {
            0 => None,
            1 => Some(reader.array::<32>()?),
            _ => return Err(WireError::InvalidEncoding),
        };
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self {
            version,
            parent_root,
            new_root,
            receipts,
            recovery_commitment,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecordV1 {
    pub version: u8,
    pub service_id: u32,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub code_hash: StateRoot,
    pub state_delta: StateDiffV1,
}

impl RecoveryRecordV1 {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.u32(self.service_id);
        writer.raw(&self.parent_root);
        writer.raw(&self.new_root);
        writer.raw(&self.code_hash);
        writer.bytes_u32(&self.state_delta.encode()?)?;
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != RECOVERY_FORMAT_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let service_id = reader.u32()?;
        let parent_root = reader.array::<32>()?;
        let new_root = reader.array::<32>()?;
        let code_hash = reader.array::<32>()?;
        let state_delta = StateDiffV1::decode(&reader.bytes_u32()?)?;
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self {
            version,
            service_id,
            parent_root,
            new_root,
            code_hash,
            state_delta,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalStateWitnessV1 {
    pub service_id: u32,
    pub state_root: StateRoot,
    pub proof: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateQueryResponseV1 {
    pub service_id: u32,
    pub state_root: StateRoot,
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>,
    pub proof: Vec<Vec<u8>>,
}

pub fn is_reserved_service_storage_key(key: &[u8]) -> bool {
    key == MANAGED_STATE_COMMITMENT_KEY_V1
}

pub trait ServiceApplication {
    type Error;

    fn execute(&self, context: &mut ExecutionContext<'_>, input: &[u8]) -> Result<(), Self::Error>;
}

pub struct ExecutionContext<'a> {
    state: &'a mut dyn ManagedStateAccess,
    sender: Option<[u8; 32]>,
}

impl<'a> ExecutionContext<'a> {
    pub fn new(state: &'a mut dyn ManagedStateAccess, sender: Option<[u8; 32]>) -> Self {
        Self { state, sender }
    }

    pub fn state(&mut self) -> &mut dyn ManagedStateAccess {
        self.state
    }

    pub fn sender(&self) -> Option<[u8; 32]> {
        self.sender
    }

    pub fn begin_transaction(&mut self) -> Result<(), StateAccessError> {
        self.state.begin_transaction()
    }

    pub fn commit_transaction(&mut self) -> Result<(), StateAccessError> {
        self.state.commit_transaction()
    }

    pub fn rollback_transaction(&mut self) -> Result<(), StateAccessError> {
        self.state.rollback_transaction()
    }
}

pub trait ManagedStateAccess {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError>;
    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), StateAccessError>;
    fn delete(&mut self, key: &[u8]) -> Result<(), StateAccessError>;

    fn begin_transaction(&mut self) -> Result<(), StateAccessError> {
        Err(StateAccessError::Backend)
    }

    fn commit_transaction(&mut self) -> Result<(), StateAccessError> {
        Err(StateAccessError::Backend)
    }

    fn rollback_transaction(&mut self) -> Result<(), StateAccessError> {
        Err(StateAccessError::Backend)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateAccessError {
    MissingWitness,
    InvalidProof,
    Backend,
    ReservedKey,
    ApplicationFailed(u32),
}

pub trait RawJamStorage {
    fn read(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, RawJamError>;
    fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), RawJamError>;
    fn delete(&mut self, key: &[u8]) -> Result<(), RawJamError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawJamError {
    Host,
    ReservedKey,
}

pub struct RefineContext<'a> {
    managed: &'a mut dyn ManagedStateAccess,
}

impl<'a> RefineContext<'a> {
    pub fn new(managed: &'a mut dyn ManagedStateAccess) -> Self {
        Self { managed }
    }

    pub fn managed_state(&mut self) -> &mut dyn ManagedStateAccess {
        self.managed
    }
}

pub struct AccumulateContext<'a, S: RawJamStorage> {
    storage: &'a mut S,
}

impl<'a, S: RawJamStorage> AccumulateContext<'a, S> {
    pub fn new(storage: &'a mut S) -> Self {
        Self { storage }
    }

    pub fn read(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, RawJamError> {
        self.storage.read(key)
    }

    pub fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), RawJamError> {
        if is_reserved_service_storage_key(key) {
            return Err(RawJamError::ReservedKey);
        }
        self.storage.write(key, value)
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<(), RawJamError> {
        if is_reserved_service_storage_key(key) {
            return Err(RawJamError::ReservedKey);
        }
        self.storage.delete(key)
    }

    /// Commit a verified managed-state transition through the runtime-owned key.
    /// Ordinary raw storage callers cannot access this namespace through `write`.
    pub fn commit_managed_state(&mut self, root: StateRoot) -> Result<(), RawJamError> {
        let commitment = ManagedStateCommitmentV1::new(root).encode();
        self.storage
            .write(MANAGED_STATE_COMMITMENT_KEY_V1, &commitment)
    }
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn raw(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    fn bytes_u32(&mut self, value: &[u8]) -> Result<(), WireError> {
        let length = u32::try_from(value.len()).map_err(|_| WireError::LengthOverflow)?;
        self.u32(length);
        self.raw(value);
        Ok(())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(WireError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(WireError::UnexpectedEof)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(*self.take(1)?.first().ok_or(WireError::UnexpectedEof)?)
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array::<4>()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.take(N)?
            .try_into()
            .map_err(|_| WireError::InvalidLength)
    }

    fn bytes_u32(&mut self) -> Result<Vec<u8>, WireError> {
        let length = self.u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }

    fn bytes_limited(&mut self, maximum: usize) -> Result<Vec<u8>, WireError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(WireError::TooManyItems);
        }
        Ok(self.take(length)?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[derive(Default)]
    struct TestStorage {
        writes: Vec<(Vec<u8>, Vec<u8>)>,
    }

    impl RawJamStorage for TestStorage {
        fn read(&mut self, _key: &[u8]) -> Result<Option<Vec<u8>>, RawJamError> {
            Ok(None)
        }

        fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), RawJamError> {
            self.writes.push((key.to_vec(), value.to_vec()));
            Ok(())
        }

        fn delete(&mut self, _key: &[u8]) -> Result<(), RawJamError> {
            Ok(())
        }
    }

    #[test]
    fn commitment_is_exactly_34_bytes() {
        let commitment = ManagedStateCommitmentV1::new([7; 32]);
        assert_eq!(commitment.encode().len(), 34);
        assert_eq!(
            ManagedStateCommitmentV1::decode(&commitment.encode()),
            Ok(commitment)
        );
        assert_eq!(
            ManagedStateCommitmentV1::decode(&[0; 33]),
            Err(WireError::InvalidLength)
        );
        assert_eq!(ManagedStateCommitmentV1::EMPTY.root, EMPTY_STATE_ROOT_V1);
    }

    #[test]
    fn keys_are_unhashed_and_service_local() {
        assert_eq!(
            application_key_v1(b"best-score/v1", &[9; 4]).unwrap(),
            [vec![1, 13, 0], b"best-score/v1".to_vec(), vec![9; 4]].concat()
        );
        assert_eq!(wallet_nonce_key_v1(&[3; 32])[..2], [0, 1]);
    }

    #[test]
    fn diff_is_canonical_and_rejects_duplicates() {
        let mut diff = StateDiffV1 {
            changes: vec![
                StateChangeV1 {
                    key: vec![2],
                    value: None,
                },
                StateChangeV1 {
                    key: vec![1],
                    value: Some(vec![3]),
                },
            ],
        };
        let encoded = diff.encode().unwrap();
        assert_eq!(
            StateDiffV1::decode(&encoded).unwrap().changes[0].key,
            vec![1]
        );
        diff.changes.push(StateChangeV1 {
            key: vec![1],
            value: None,
        });
        assert_eq!(diff.encode(), Err(WireError::DuplicateKey));
    }

    #[test]
    fn transition_and_witness_round_trip_without_layout_leaks() {
        let transition = StateTransitionV1 {
            version: 1,
            parent_root: [1; 32],
            new_root: [2; 32],
            diff_hash: [3; 32],
        };
        assert_eq!(
            StateTransitionV1::decode(&transition.encode()),
            Ok(transition)
        );
        let mut invalid_transition = transition.encode();
        invalid_transition[0] = 2;
        assert_eq!(
            StateTransitionV1::decode(&invalid_transition),
            Err(WireError::UnsupportedVersion)
        );
        let witness = ManagedStateWitnessV1 {
            version: 1,
            parent_root: [4; 32],
            storage_proof: vec![vec![5, 6]],
        };
        assert_eq!(
            ManagedStateWitnessV1::decode(&witness.encode().unwrap()),
            Ok(witness)
        );
    }

    #[test]
    fn only_runtime_commit_path_can_write_the_reserved_key() {
        let mut storage = TestStorage::default();
        {
            let mut context = AccumulateContext::new(&mut storage);
            assert_eq!(
                context.write(MANAGED_STATE_COMMITMENT_KEY_V1, &[1]),
                Err(RawJamError::ReservedKey)
            );
            context.commit_managed_state([7; 32]).unwrap();
        }
        assert_eq!(storage.writes.len(), 1);
        assert_eq!(storage.writes[0].0, MANAGED_STATE_COMMITMENT_KEY_V1);
        assert_eq!(
            ManagedStateCommitmentV1::decode(&storage.writes[0].1),
            Ok(ManagedStateCommitmentV1::new([7; 32]))
        );
    }

    #[test]
    fn witness_decode_rejects_untrusted_allocation_sizes() {
        let mut encoded = vec![1];
        encoded.extend_from_slice(&[0; 32]);
        encoded.extend_from_slice(&((MAX_WITNESS_NODES as u32) + 1).to_le_bytes());
        assert_eq!(
            ManagedStateWitnessV1::decode(&encoded),
            Err(WireError::TooManyItems)
        );

        let oversized = ManagedStateWitnessV1 {
            version: 1,
            parent_root: [0; 32],
            storage_proof: vec![vec![0; MAX_WITNESS_NODE_BYTES + 1]],
        };
        assert_eq!(oversized.encode(), Err(WireError::TooManyItems));
    }
}
