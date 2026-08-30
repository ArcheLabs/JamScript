#![no_std]

extern crate alloc;

use alloc::vec::Vec;
pub use jamscript_crypto::blake2_256;

pub type StateRoot = [u8; 32];

pub const MANAGED_STATE_PROTOCOL_VERSION: u8 = 1;
pub const MANAGED_STATE_LAYOUT_VERSION: u8 = 1;
pub const RECOVERY_FORMAT_VERSION: u8 = 1;
pub const STATE_VIEW_VERSION: u8 = 1;
pub const SCRIPT_ACTION_RESULT_VERSION: u8 = 1;
pub const EMPTY_STATE_ROOT_V1: StateRoot = [
    0x03, 0x17, 0x0a, 0x2e, 0x75, 0x97, 0xb7, 0xb7, 0xe3, 0xd8, 0x4c, 0x05, 0x39, 0x1d, 0x13, 0x9a,
    0x62, 0xb1, 0x57, 0xe7, 0x87, 0x86, 0xd8, 0xc0, 0x82, 0xf2, 0x9d, 0xcf, 0x4c, 0x11, 0x13, 0x14,
];
pub const MANAGED_STATE_COMMITMENT_KEY_V1: &[u8] = b":jam-service-runtime:managed-state:v1";
/// Runtime-owned JamScript management metadata.  Applications may read these
/// keys when the execution path needs them, but may never mutate them through
/// the ordinary managed-state API.
pub const MANAGEMENT_VERSION_KEY_V1: &[u8] = b"__jamscript/management/version";
pub const MANAGEMENT_INITIALIZED_KEY_V1: &[u8] = b"__jamscript/management/initialized";
pub const MANAGEMENT_POLICY_KEY_V1: &[u8] = b"__jamscript/management/policy";
pub const MANAGEMENT_NONCE_KEY_V1: &[u8] = b"__jamscript/management/nonce";
pub const APPLICATION_KEY_CLASS_V1: u8 = 0x01;
pub const RUNTIME_KEY_CLASS_V1: u8 = 0x00;
pub const WALLET_AUTH_MODULE_V1: u8 = 0x01;
pub const MAX_RUNTIME_ACTIONS: usize = 1024;
pub const MAX_RECOVERY_CHANGES: usize = 4096;
pub const MAX_RECOVERY_BYTES: usize = 1024 * 1024;
pub const MAX_STATE_KEY_BYTES: usize = 4096;
pub const MAX_STATE_VALUE_BYTES: usize = 64 * 1024;
pub const MAX_STATE_VIEW_ENTRIES: usize = MAX_RECOVERY_CHANGES;
pub const MAX_STATE_VIEW_BYTES: usize = 1024 * 1024;
pub const MAX_SCRIPT_ACTION_RESULT_BYTES: usize = MAX_RECOVERY_BYTES;
pub const MAX_WITNESS_NODES: usize = 4096;
pub const MAX_WITNESS_NODE_BYTES: usize = 64 * 1024;
pub const MAX_WITNESS_BYTES: usize = 1024 * 1024;
pub const MAX_WITNESS_ENCODED_BYTES: usize =
    1 + 32 + 4 + (MAX_WITNESS_NODES * 4) + MAX_WITNESS_BYTES;
pub const MAX_ACCESS_PLAN_ENCODED_BYTES: usize = MAX_STATE_VIEW_BYTES;
pub const MAX_WITNESS_V1_ENCODED_BYTES: usize =
    1 + 32 + 4 + MAX_ACCESS_PLAN_ENCODED_BYTES + 4 + (MAX_WITNESS_NODES * 4) + MAX_WITNESS_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ServiceKeyV1([u8; 32]);

impl ServiceKeyV1 {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != 32 {
            return Err(WireError::InvalidLength);
        }
        Ok(Self(
            bytes.try_into().map_err(|_| WireError::InvalidLength)?,
        ))
    }
}

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
    ReservedKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateViewEntryV1 {
    pub key: Vec<u8>,
    /// `None` means the key was proven absent. An omitted key is not part of
    /// the view at all and must be reported by the application runtime as
    /// `NeedState`.
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateViewV1 {
    pub version: u8,
    pub entries: Vec<StateViewEntryV1>,
}

impl Default for StateViewV1 {
    fn default() -> Self {
        Self {
            version: STATE_VIEW_VERSION,
            entries: Vec::new(),
        }
    }
}

impl StateViewV1 {
    pub fn from_entries<I>(entries: I) -> Result<Self, WireError>
    where
        I: IntoIterator<Item = StateViewEntryV1>,
    {
        let mut entries = entries.into_iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        let view = Self {
            version: STATE_VIEW_VERSION,
            entries,
        };
        view.validate_canonical()?;
        Ok(view)
    }

    pub fn get(&self, key: &[u8]) -> Option<&Option<Vec<u8>>> {
        self.entries
            .binary_search_by(|entry| entry.key.as_slice().cmp(key))
            .ok()
            .map(|index| &self.entries[index].value)
    }

    fn validate_canonical(&self) -> Result<(), WireError> {
        if self.version != STATE_VIEW_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        if self.entries.len() > MAX_STATE_VIEW_ENTRIES {
            return Err(WireError::TooManyItems);
        }
        for entry in &self.entries {
            if entry.key.len() > MAX_STATE_KEY_BYTES
                || entry
                    .value
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_STATE_VALUE_BYTES)
            {
                return Err(WireError::TooManyItems);
            }
        }
        for pair in self.entries.windows(2) {
            if pair[0].key == pair[1].key {
                return Err(WireError::DuplicateKey);
            }
            if pair[0].key > pair[1].key {
                return Err(WireError::UnsortedKeys);
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.validate_canonical()?;
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.u32(u32::try_from(self.entries.len()).map_err(|_| WireError::LengthOverflow)?);
        for entry in &self.entries {
            writer.bytes_u32(&entry.key)?;
            match &entry.value {
                None => writer.u8(0),
                Some(value) => {
                    writer.u8(1);
                    writer.bytes_u32(value)?;
                }
            }
        }
        let encoded = writer.finish();
        if encoded.len() > MAX_STATE_VIEW_BYTES {
            return Err(WireError::TooManyItems);
        }
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > MAX_STATE_VIEW_BYTES {
            return Err(WireError::TooManyItems);
        }
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != STATE_VIEW_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let count = reader.u32()? as usize;
        if count > MAX_STATE_VIEW_ENTRIES {
            return Err(WireError::TooManyItems);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let key = reader.bytes_limited(MAX_STATE_KEY_BYTES)?;
            let value = match reader.u8()? {
                0 => None,
                1 => Some(reader.bytes_limited(MAX_STATE_VALUE_BYTES)?),
                _ => return Err(WireError::InvalidEncoding),
            };
            entries.push(StateViewEntryV1 { key, value });
        }
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        let view = Self { version, entries };
        view.validate_canonical()?;
        Ok(view)
    }
}

pub const SCRIPT_ACTION_KIND_APPLIED: u8 = 0;
pub const SCRIPT_ACTION_KIND_ABORT: u8 = 1;
pub const SCRIPT_ACTION_KIND_NEED_STATE: u8 = 2;
pub const SCRIPT_ACTION_KIND_FATAL: u8 = 3;
pub const MAX_APPLICATION_ABORT_CODE: u32 = 0x00ff_ffff;
pub const MIN_FATAL_ERROR_CODE: u32 = 0x8000_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptActionResultV1 {
    Applied(StateDiffV1),
    Abort(u32),
    NeedState(Vec<u8>),
    Fatal(u32),
}

impl ScriptActionResultV1 {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.u8(SCRIPT_ACTION_RESULT_VERSION);
        match self {
            Self::Applied(diff) => {
                validate_application_diff(diff)?;
                writer.u8(SCRIPT_ACTION_KIND_APPLIED);
                writer.bytes_u32(&diff.encode()?)?;
            }
            Self::Abort(code) if (1..=MAX_APPLICATION_ABORT_CODE).contains(code) => {
                writer.u8(SCRIPT_ACTION_KIND_ABORT);
                writer.u32(*code);
            }
            Self::NeedState(key) if key.len() <= MAX_STATE_KEY_BYTES => {
                writer.u8(SCRIPT_ACTION_KIND_NEED_STATE);
                writer.bytes_u32(key)?;
            }
            Self::Fatal(code) if *code >= MIN_FATAL_ERROR_CODE => {
                writer.u8(SCRIPT_ACTION_KIND_FATAL);
                writer.u32(*code);
            }
            _ => return Err(WireError::InvalidEncoding),
        }
        let encoded = writer.finish();
        if encoded.len() > MAX_SCRIPT_ACTION_RESULT_BYTES {
            return Err(WireError::TooManyItems);
        }
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > MAX_SCRIPT_ACTION_RESULT_BYTES {
            return Err(WireError::TooManyItems);
        }
        let mut reader = Reader::new(bytes);
        if reader.u8()? != SCRIPT_ACTION_RESULT_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let result = match reader.u8()? {
            SCRIPT_ACTION_KIND_APPLIED => {
                let diff = StateDiffV1::decode(&reader.bytes_limited(MAX_RECOVERY_BYTES)?)?;
                validate_application_diff(&diff)?;
                Self::Applied(diff)
            }
            SCRIPT_ACTION_KIND_ABORT => {
                let code = reader.u32()?;
                if !(1..=MAX_APPLICATION_ABORT_CODE).contains(&code) {
                    return Err(WireError::InvalidEncoding);
                }
                Self::Abort(code)
            }
            SCRIPT_ACTION_KIND_NEED_STATE => {
                Self::NeedState(reader.bytes_limited(MAX_STATE_KEY_BYTES)?)
            }
            SCRIPT_ACTION_KIND_FATAL => {
                let code = reader.u32()?;
                if code < MIN_FATAL_ERROR_CODE {
                    return Err(WireError::InvalidEncoding);
                }
                Self::Fatal(code)
            }
            _ => return Err(WireError::InvalidEncoding),
        };
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(result)
    }
}

fn validate_application_diff(diff: &StateDiffV1) -> Result<(), WireError> {
    diff.validate_canonical()?;
    if diff
        .changes
        .iter()
        .any(|change| change.key.first() != Some(&APPLICATION_KEY_CLASS_V1))
    {
        return Err(WireError::ReservedKey);
    }
    Ok(())
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
    pub fn validate_limits(&self) -> Result<(), WireError> {
        if self.changes.len() > MAX_RECOVERY_CHANGES {
            return Err(WireError::TooManyItems);
        }
        for change in &self.changes {
            if is_reserved_service_storage_key(&change.key) {
                return Err(WireError::ReservedKey);
            }
            if change.key.len() > MAX_STATE_KEY_BYTES {
                return Err(WireError::TooManyItems);
            }
            if change
                .value
                .as_ref()
                .is_some_and(|value| value.len() > MAX_STATE_VALUE_BYTES)
            {
                return Err(WireError::TooManyItems);
            }
        }
        Ok(())
    }

    pub fn canonicalize(&mut self) -> Result<(), WireError> {
        self.changes.sort_by(|left, right| left.key.cmp(&right.key));
        self.validate_canonical()
    }

    fn validate_canonical(&self) -> Result<(), WireError> {
        self.validate_limits()?;
        for pair in self.changes.windows(2) {
            if pair[0].key == pair[1].key {
                return Err(WireError::DuplicateKey);
            }
            if pair[0].key > pair[1].key {
                return Err(WireError::UnsortedKeys);
            }
        }
        Ok(())
    }

    fn encode_canonical_checked(&self) -> Result<Vec<u8>, WireError> {
        self.validate_canonical()?;
        let count = u32::try_from(self.changes.len()).map_err(|_| WireError::LengthOverflow)?;
        let mut writer = Writer::new();
        writer.u8(MANAGED_STATE_PROTOCOL_VERSION);
        writer.u32(count);
        for change in &self.changes {
            writer.bytes_u32(&change.key)?;
            match &change.value {
                Some(value) => {
                    writer.u8(1);
                    writer.bytes_u32(value)?;
                }
                None => writer.u8(0),
            }
        }
        Ok(writer.finish())
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.validate_limits()?;
        let mut canonical = self.clone();
        canonical.canonicalize()?;
        canonical.encode_canonical_checked()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        if reader.u8()? != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let count = reader.u32()? as usize;
        if count > MAX_RECOVERY_CHANGES {
            return Err(WireError::TooManyItems);
        }
        let mut changes = Vec::with_capacity(count);
        for _ in 0..count {
            let key = reader.bytes_limited(MAX_STATE_KEY_BYTES)?;
            let value = match reader.u8()? {
                0 => None,
                1 => Some(reader.bytes_limited(MAX_STATE_VALUE_BYTES)?),
                _ => return Err(WireError::InvalidEncoding),
            };
            changes.push(StateChangeV1 { key, value });
        }
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        let diff = Self { changes };
        diff.validate_limits()?;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateRecoveryV1 {
    pub version: u8,
    pub diff: StateDiffV1,
}

impl StateRecoveryV1 {
    pub fn new(diff: StateDiffV1) -> Result<Self, WireError> {
        let mut diff = diff;
        diff.validate_limits()?;
        diff.canonicalize()?;
        Ok(Self {
            version: RECOVERY_FORMAT_VERSION,
            diff,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.version != RECOVERY_FORMAT_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let diff = self.diff.encode_canonical_checked()?;
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.bytes_u32(&diff)?;
        let encoded = writer.finish();
        if encoded.len() > MAX_RECOVERY_BYTES {
            return Err(WireError::TooManyItems);
        }
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > MAX_RECOVERY_BYTES {
            return Err(WireError::TooManyItems);
        }
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != RECOVERY_FORMAT_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let diff = StateDiffV1::decode(&reader.bytes_limited(MAX_RECOVERY_BYTES)?)?;
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self { version, diff })
    }

    pub fn commitment(&self) -> Result<StateRoot, WireError> {
        Ok(blake2_256(&self.encode()?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateAccessPlanV1 {
    pub version: u8,
    pub keys: Vec<Vec<u8>>,
}

impl StateAccessPlanV1 {
    pub fn for_wallet<I, K>(sender: &[u8; 32], application_keys: I) -> Result<Self, WireError>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<[u8]>,
    {
        let nonce_key = wallet_nonce_key_v1(sender);
        let keys = core::iter::once(nonce_key).chain(
            application_keys
                .into_iter()
                .map(|key| key.as_ref().to_vec()),
        );
        Self::from_keys(keys)
    }

    pub fn for_public<I, K>(application_keys: I) -> Result<Self, WireError>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<[u8]>,
    {
        Self::from_keys(application_keys)
    }

    pub fn from_keys<I, K>(keys: I) -> Result<Self, WireError>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<[u8]>,
    {
        let mut keys = keys
            .into_iter()
            .map(|key| key.as_ref().to_vec())
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        if keys.len() > MAX_RECOVERY_CHANGES
            || keys.iter().any(|key| key.len() > MAX_STATE_KEY_BYTES)
        {
            return Err(WireError::TooManyItems);
        }
        Ok(Self {
            version: MANAGED_STATE_PROTOCOL_VERSION,
            keys,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.version != MANAGED_STATE_PROTOCOL_VERSION || self.keys.len() > MAX_RECOVERY_CHANGES
        {
            return Err(WireError::TooManyItems);
        }
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.u32(self.keys.len() as u32);
        for key in &self.keys {
            if key.len() > MAX_STATE_KEY_BYTES {
                return Err(WireError::TooManyItems);
            }
            writer.bytes_u32(key)?;
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let count = reader.u32()? as usize;
        if count > MAX_RECOVERY_CHANGES {
            return Err(WireError::TooManyItems);
        }
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            keys.push(reader.bytes_limited(MAX_STATE_KEY_BYTES)?);
        }
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        let canonical = Self::from_keys(&keys)?;
        if canonical.keys != keys {
            return Err(WireError::UnsortedKeys);
        }
        Ok(Self { version, keys })
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
    pub access_plan: StateAccessPlanV1,
    pub storage_proof: Vec<Vec<u8>>,
}

impl ManagedStateWitnessV1 {
    pub const VERSION: u8 = 1;

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.version != Self::VERSION || self.storage_proof.len() > MAX_WITNESS_NODES {
            return Err(WireError::UnsupportedVersion);
        }
        let plan = self.access_plan.encode()?;
        if plan.len() > MAX_ACCESS_PLAN_ENCODED_BYTES {
            return Err(WireError::TooManyItems);
        }
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.raw(&self.parent_root);
        writer.bytes_u32(&plan)?;
        writer.u32(u32::try_from(self.storage_proof.len()).map_err(|_| WireError::LengthOverflow)?);
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
        if bytes.len() > MAX_WITNESS_V1_ENCODED_BYTES {
            return Err(WireError::TooManyItems);
        }
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != Self::VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let parent_root = reader.array::<32>()?;
        let access_plan =
            StateAccessPlanV1::decode(&reader.bytes_limited(MAX_ACCESS_PLAN_ENCODED_BYTES)?)?;
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
            access_plan,
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
    pub const VERSION: u8 = 1;

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.version != Self::VERSION || self.actions.len() > MAX_RUNTIME_ACTIONS {
            return Err(WireError::UnsupportedVersion);
        }
        let witness = self.managed_state.encode()?;
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.bytes_u32(&witness)?;
        writer.u32(u32::try_from(self.actions.len()).map_err(|_| WireError::LengthOverflow)?);
        for action in &self.actions {
            writer.bytes_u32(action)?;
        }
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != Self::VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let managed_state =
            ManagedStateWitnessV1::decode(&reader.bytes_limited(MAX_WITNESS_V1_ENCODED_BYTES)?)?;
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
            managed_state,
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
    pub transition_valid_until: Option<u64>,
    pub receipts: Vec<ActionReceiptV1>,
    pub recovery_commitment: StateRoot,
    pub recovery_payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeRefineTransitionHeaderV1 {
    pub version: u8,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub transition_valid_until: Option<u64>,
    pub recovery_commitment: StateRoot,
}

impl RuntimeRefineOutputV1 {
    pub fn from_diff(
        parent_root: StateRoot,
        new_root: StateRoot,
        receipts: Vec<ActionReceiptV1>,
        diff: StateDiffV1,
    ) -> Result<Self, WireError> {
        Self::from_diff_with_validity(parent_root, new_root, receipts, diff, None)
    }

    pub fn from_diff_with_validity(
        parent_root: StateRoot,
        new_root: StateRoot,
        receipts: Vec<ActionReceiptV1>,
        diff: StateDiffV1,
        transition_valid_until: Option<u64>,
    ) -> Result<Self, WireError> {
        let recovery = StateRecoveryV1::new(diff)?;
        let recovery_payload = recovery.encode()?;
        let recovery_commitment = blake2_256(&recovery_payload);
        Ok(Self {
            version: MANAGED_STATE_PROTOCOL_VERSION,
            parent_root,
            new_root,
            transition_valid_until,
            receipts,
            recovery_commitment,
            recovery_payload,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.version != MANAGED_STATE_PROTOCOL_VERSION
            || self.receipts.len() > MAX_RUNTIME_ACTIONS
            || self.recovery_payload.len() > MAX_RECOVERY_BYTES
        {
            return Err(WireError::TooManyItems);
        }
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.raw(&self.parent_root);
        writer.raw(&self.new_root);
        match self.transition_valid_until {
            Some(valid_until) => {
                writer.u8(1);
                writer.raw(&valid_until.to_le_bytes());
            }
            None => writer.u8(0),
        }
        writer.raw(&self.recovery_commitment);
        writer.u32(self.receipts.len() as u32);
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
        writer.bytes_u32(&self.recovery_payload)?;
        Ok(writer.finish())
    }

    pub fn decode_transition_header(
        bytes: &[u8],
    ) -> Result<RuntimeRefineTransitionHeaderV1, WireError> {
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let parent_root = reader.array::<32>()?;
        let new_root = reader.array::<32>()?;
        let transition_valid_until = match reader.u8()? {
            0 => None,
            1 => Some(reader.u64()?),
            _ => return Err(WireError::InvalidEncoding),
        };
        let recovery_commitment = reader.array::<32>()?;
        let count = reader.u32()? as usize;
        if count > MAX_RUNTIME_ACTIONS {
            return Err(WireError::TooManyItems);
        }
        for _ in 0..count {
            let _ = reader.array::<32>()?;
            let status = reader.u8()?;
            if status > ActionStatusV1::Rejected as u8 {
                return Err(WireError::InvalidEncoding);
            }
            match reader.u8()? {
                0 => {}
                1 => {
                    let _ = reader.u32()?;
                }
                _ => return Err(WireError::InvalidEncoding),
            }
        }
        Ok(RuntimeRefineTransitionHeaderV1 {
            version,
            parent_root,
            new_root,
            transition_valid_until,
            recovery_commitment,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > MAX_RECOVERY_BYTES + 128 * MAX_RUNTIME_ACTIONS {
            return Err(WireError::TooManyItems);
        }
        let mut reader = Reader::new(bytes);
        let version = reader.u8()?;
        if version != MANAGED_STATE_PROTOCOL_VERSION {
            return Err(WireError::UnsupportedVersion);
        }
        let parent_root = reader.array::<32>()?;
        let new_root = reader.array::<32>()?;
        let transition_valid_until = match reader.u8()? {
            0 => None,
            1 => Some(reader.u64()?),
            _ => return Err(WireError::InvalidEncoding),
        };
        let recovery_commitment = reader.array::<32>()?;
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
        let recovery_payload = reader.bytes_limited(MAX_RECOVERY_BYTES)?;
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        let output = Self {
            version,
            parent_root,
            new_root,
            transition_valid_until,
            receipts,
            recovery_commitment,
            recovery_payload,
        };
        if blake2_256(&output.recovery_payload) != output.recovery_commitment {
            return Err(WireError::InvalidEncoding);
        }
        StateRecoveryV1::decode(&output.recovery_payload)?;
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecordV1 {
    pub version: u8,
    pub service_key: ServiceKeyV1,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub code_hash: StateRoot,
    pub state_delta: StateDiffV1,
}

impl RecoveryRecordV1 {
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.raw(self.service_key.as_bytes());
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
        let service_key = ServiceKeyV1::new(reader.array::<32>()?);
        let parent_root = reader.array::<32>()?;
        let new_root = reader.array::<32>()?;
        let code_hash = reader.array::<32>()?;
        let state_delta = StateDiffV1::decode(&reader.bytes_u32()?)?;
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self {
            version,
            service_key,
            parent_root,
            new_root,
            code_hash,
            state_delta,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalStateWitnessV1 {
    pub service_key: ServiceKeyV1,
    pub state_root: StateRoot,
    pub proof: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateQueryResponseV1 {
    pub service_key: ServiceKeyV1,
    pub state_root: StateRoot,
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>,
    pub proof: Vec<Vec<u8>>,
}

pub fn is_reserved_service_storage_key(key: &[u8]) -> bool {
    matches!(
        key,
        MANAGED_STATE_COMMITMENT_KEY_V1
            | MANAGEMENT_VERSION_KEY_V1
            | MANAGEMENT_INITIALIZED_KEY_V1
            | MANAGEMENT_POLICY_KEY_V1
            | MANAGEMENT_NONCE_KEY_V1
    )
}

pub trait ServiceApplication {
    type Error;

    fn execute(&self, context: &mut ExecutionContext<'_>, input: &[u8]) -> Result<(), Self::Error>;
}

pub struct ExecutionContext<'a> {
    state: &'a mut dyn ManagedStateAccess,
    access_plan: Option<&'a StateAccessPlanV1>,
    sender: Option<[u8; 32]>,
    transition_valid_until: Option<u64>,
}

impl<'a> ExecutionContext<'a> {
    pub fn new(state: &'a mut dyn ManagedStateAccess, sender: Option<[u8; 32]>) -> Self {
        Self {
            state,
            access_plan: None,
            sender,
            transition_valid_until: None,
        }
    }

    pub fn with_access_plan(
        state: &'a mut dyn ManagedStateAccess,
        sender: Option<[u8; 32]>,
        access_plan: &'a StateAccessPlanV1,
    ) -> Self {
        Self {
            state,
            access_plan: Some(access_plan),
            sender,
            transition_valid_until: None,
        }
    }

    pub fn state(&mut self) -> &mut dyn ManagedStateAccess {
        self.state
    }

    pub fn state_view(&mut self) -> Result<StateViewV1, StateAccessError> {
        let plan = self.access_plan.ok_or(StateAccessError::Backend)?;
        let mut entries = Vec::new();
        for key in &plan.keys {
            if key.first() != Some(&APPLICATION_KEY_CLASS_V1) {
                continue;
            }
            entries.push(StateViewEntryV1 {
                key: key.clone(),
                value: self.state.get(key)?,
            });
        }
        StateViewV1::from_entries(entries).map_err(|_| StateAccessError::Backend)
    }

    pub fn sender(&self) -> Option<[u8; 32]> {
        self.sender
    }

    pub fn constrain_valid_until(&mut self, valid_until: u64) {
        self.transition_valid_until = Some(
            self.transition_valid_until
                .map_or(valid_until, |current| current.min(valid_until)),
        );
    }

    pub fn transition_valid_until(&self) -> Option<u64> {
        self.transition_valid_until
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateAccessError {
    MissingWitness,
    InvalidProof,
    Backend,
    ReservedKey,
    NeedState(Vec<u8>),
    Rejected(u32),
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

    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array::<8>()?))
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
            access_plan: StateAccessPlanV1::from_keys(core::iter::empty::<&[u8]>()).unwrap(),
            storage_proof: vec![vec![5, 6]],
        };
        assert_eq!(
            ManagedStateWitnessV1::decode(&witness.encode().unwrap()),
            Ok(witness)
        );

        let witness_v1 = ManagedStateWitnessV1 {
            version: ManagedStateWitnessV1::VERSION,
            parent_root: [4; 32],
            access_plan: StateAccessPlanV1::from_keys([b"a".as_slice(), b"b"]).unwrap(),
            storage_proof: vec![vec![5, 6]],
        };
        assert_eq!(
            ManagedStateWitnessV1::decode(&witness_v1.encode().unwrap()),
            Ok(witness_v1.clone())
        );
        let input_v1 = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: witness_v1,
            actions: vec![vec![7]],
        };
        assert_eq!(
            RuntimeRefineInputV1::decode(&input_v1.encode().unwrap()),
            Ok(input_v1)
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
        encoded.extend_from_slice(&5u32.to_le_bytes());
        encoded.extend_from_slice(&[1, 0, 0, 0, 0]);
        encoded.extend_from_slice(&((MAX_WITNESS_NODES as u32) + 1).to_le_bytes());
        assert_eq!(
            ManagedStateWitnessV1::decode(&encoded),
            Err(WireError::TooManyItems)
        );

        let oversized = ManagedStateWitnessV1 {
            version: 1,
            parent_root: [0; 32],
            access_plan: StateAccessPlanV1::from_keys(core::iter::empty::<&[u8]>()).unwrap(),
            storage_proof: vec![vec![0; MAX_WITNESS_NODE_BYTES + 1]],
        };
        assert_eq!(oversized.encode(), Err(WireError::TooManyItems));
    }

    #[test]
    fn recovery_and_access_plan_are_canonical_and_bounded() {
        let diff = StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: b"counter".to_vec(),
                value: Some(vec![1]),
            }],
        };
        let recovery = StateRecoveryV1::new(diff).unwrap();
        let encoded = recovery.encode().unwrap();
        assert_eq!(StateRecoveryV1::decode(&encoded).unwrap(), recovery);
        assert_eq!(recovery.commitment().unwrap(), blake2_256(&encoded));

        let plan = StateAccessPlanV1::from_keys([b"b".as_slice(), b"a", b"a"]).unwrap();
        assert_eq!(plan.keys, vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(
            StateAccessPlanV1::decode(&plan.encode().unwrap()).unwrap(),
            plan
        );
        let wallet_plan = StateAccessPlanV1::for_wallet(&[7; 32], [b"counter".as_slice()]).unwrap();
        assert!(wallet_plan.keys.contains(&wallet_nonce_key_v1(&[7; 32])));

        let oversized = StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: vec![0; MAX_STATE_KEY_BYTES + 1],
                value: None,
            }],
        };
        assert_eq!(
            StateRecoveryV1::new(oversized),
            Err(WireError::TooManyItems)
        );
    }

    #[test]
    fn state_view_distinguishes_absence_from_omission_and_is_canonical() {
        let view = StateViewV1::from_entries([
            StateViewEntryV1 {
                key: vec![2],
                value: Some(vec![9]),
            },
            StateViewEntryV1 {
                key: vec![1],
                value: None,
            },
        ])
        .unwrap();
        assert_eq!(view.get(&[1]), Some(&None));
        assert_eq!(view.get(&[2]), Some(&Some(vec![9])));
        assert_eq!(view.get(&[3]), None);
        assert_eq!(StateViewV1::decode(&view.encode().unwrap()), Ok(view));

        let duplicate = StateViewV1 {
            version: STATE_VIEW_VERSION,
            entries: vec![
                StateViewEntryV1 {
                    key: vec![1],
                    value: None,
                },
                StateViewEntryV1 {
                    key: vec![1],
                    value: Some(vec![2]),
                },
            ],
        };
        assert_eq!(duplicate.encode(), Err(WireError::DuplicateKey));

        let mut trailing = StateViewV1::default().encode().unwrap();
        trailing.push(0);
        assert_eq!(
            StateViewV1::decode(&trailing),
            Err(WireError::InvalidEncoding)
        );
    }

    #[test]
    fn script_action_results_are_strict_and_application_scoped() {
        let application_key = application_key_v1(b"test/v1", b"key").unwrap();
        let cases = [
            ScriptActionResultV1::Applied(StateDiffV1 {
                changes: vec![StateChangeV1 {
                    key: application_key,
                    value: Some(vec![7]),
                }],
            }),
            ScriptActionResultV1::Abort(4),
            ScriptActionResultV1::NeedState(vec![1, 2, 3]),
            ScriptActionResultV1::Fatal(MIN_FATAL_ERROR_CODE + 1),
        ];
        for case in cases {
            assert_eq!(
                ScriptActionResultV1::decode(&case.encode().unwrap()),
                Ok(case)
            );
        }

        assert_eq!(
            ScriptActionResultV1::Abort(0).encode(),
            Err(WireError::InvalidEncoding)
        );
        assert_eq!(
            ScriptActionResultV1::Fatal(MAX_APPLICATION_ABORT_CODE).encode(),
            Err(WireError::InvalidEncoding)
        );
        assert_eq!(
            ScriptActionResultV1::Applied(StateDiffV1 {
                changes: vec![StateChangeV1 {
                    key: wallet_nonce_key_v1(&[1; 32]),
                    value: Some(vec![1]),
                }],
            })
            .encode(),
            Err(WireError::ReservedKey)
        );
    }

    #[test]
    fn runtime_refine_output_v1_matches_golden_vector() {
        let diff = StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: vec![1],
                value: Some(vec![2]),
            }],
        };
        let output = RuntimeRefineOutputV1::from_diff_with_validity(
            [1; 32],
            [2; 32],
            vec![ActionReceiptV1 {
                action_hash: [3; 32],
                status: ActionStatusV1::Applied,
                error_code: None,
            }],
            diff,
            Some(42),
        )
        .unwrap();
        let encoded = output.encode().unwrap();
        assert_eq!(
            hex::encode(&encoded),
            "0101010101010101010101010101010101010101010101010101010101010101010202020202020202020202020202020202020202020202020202020202020202012a000000000000002c60e8b423b234cec3a82162cb14e733d3f4302f84dbdb69b4253052abe42c06010000000303030303030303030303030303030303030303030303030303030303030303000015000000011000000001010000000100000001010100000002"
        );
        let decoded = RuntimeRefineOutputV1::decode(&encoded).unwrap();
        assert_eq!(decoded, output);
        let header = RuntimeRefineOutputV1::decode_transition_header(&encoded).unwrap();
        assert_eq!(header.transition_valid_until, Some(42));
        assert_eq!(header.recovery_commitment, output.recovery_commitment);
    }
}
