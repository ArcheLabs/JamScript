#!/usr/bin/env python3
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one literal replacement, found {count}")
    path.write_text(text.replace(old, new, 1))


def sub_once(path: Path, pattern: str, replacement: str, flags=0) -> None:
    text = path.read_text()
    updated, count = re.subn(pattern, replacement, text, count=1, flags=flags)
    if count != 1:
        raise SystemExit(f"{path}: expected one regex replacement, found {count}: {pattern[:80]}")
    path.write_text(updated)


core = ROOT / "crates/service-runtime-core/src/lib.rs"
guest = ROOT / "crates/service-runtime-guest/src/lib.rs"
codegen = ROOT / "crates/jamscript-codegen-rust/src/lib.rs"

replace_once(
    core,
    "pub const MAX_WITNESS_V1_ENCODED_BYTES: usize =\n    1 + 32 + 4 + MAX_ACCESS_PLAN_ENCODED_BYTES + 4 + (MAX_WITNESS_NODES * 4) + MAX_WITNESS_BYTES;\n",
    "pub const MAX_WITNESS_V1_ENCODED_BYTES: usize =\n    1 + 32 + 4 + MAX_ACCESS_PLAN_ENCODED_BYTES + 4 + (MAX_WITNESS_NODES * 4) + MAX_WITNESS_BYTES;\n"
    "pub const MAX_EXTERNAL_STATE_WITNESSES: usize = 64;\n"
    "pub const MAX_EXTERNAL_STATE_BYTES: usize = 4 * 1024 * 1024;\n",
)

old_input = '''#[derive(Clone, Debug, Eq, PartialEq)]
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
'''
new_input = '''#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalStateWitnessV1 {
    pub service_id: u32,
    pub managed_state: ManagedStateWitnessV1,
}

impl ExternalStateWitnessV1 {
    pub fn state_root(&self) -> StateRoot {
        self.managed_state.parent_root
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let witness = self.managed_state.encode()?;
        let mut writer = Writer::new();
        writer.u32(self.service_id);
        writer.bytes_u32(&witness)?;
        Ok(writer.finish())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let service_id = reader.u32()?;
        let managed_state =
            ManagedStateWitnessV1::decode(&reader.bytes_limited(MAX_WITNESS_V1_ENCODED_BYTES)?)?;
        if reader.remaining() != 0 {
            return Err(WireError::InvalidEncoding);
        }
        Ok(Self {
            service_id,
            managed_state,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalStateDependencyV1 {
    pub service_id: u32,
    pub state_root: StateRoot,
}

fn validate_external_dependencies(
    dependencies: &[ExternalStateDependencyV1],
) -> Result<(), WireError> {
    if dependencies.len() > MAX_EXTERNAL_STATE_WITNESSES {
        return Err(WireError::TooManyItems);
    }
    for pair in dependencies.windows(2) {
        if pair[0].service_id == pair[1].service_id {
            return Err(WireError::DuplicateKey);
        }
        if pair[0].service_id > pair[1].service_id {
            return Err(WireError::UnsortedKeys);
        }
    }
    Ok(())
}

fn dependencies_from_external_witnesses(
    witnesses: &[ExternalStateWitnessV1],
) -> Result<Vec<ExternalStateDependencyV1>, WireError> {
    if witnesses.len() > MAX_EXTERNAL_STATE_WITNESSES {
        return Err(WireError::TooManyItems);
    }
    let mut dependencies = witnesses
        .iter()
        .map(|witness| ExternalStateDependencyV1 {
            service_id: witness.service_id,
            state_root: witness.state_root(),
        })
        .collect::<Vec<_>>();
    dependencies.sort_by_key(|dependency| dependency.service_id);
    validate_external_dependencies(&dependencies)?;
    Ok(dependencies)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRefineInputV1 {
    pub version: u8,
    pub managed_state: ManagedStateWitnessV1,
    pub external_state: Vec<ExternalStateWitnessV1>,
    pub actions: Vec<Vec<u8>>,
}

impl RuntimeRefineInputV1 {
    pub const VERSION: u8 = 1;

    pub fn external_dependencies(&self) -> Result<Vec<ExternalStateDependencyV1>, WireError> {
        dependencies_from_external_witnesses(&self.external_state)
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.version != Self::VERSION || self.actions.len() > MAX_RUNTIME_ACTIONS {
            return Err(WireError::UnsupportedVersion);
        }
        let witness = self.managed_state.encode()?;
        let dependencies = self.external_dependencies()?;
        let mut ordered_external = self.external_state.iter().collect::<Vec<_>>();
        ordered_external.sort_by_key(|witness| witness.service_id);
        if dependencies.len() != ordered_external.len() {
            return Err(WireError::InvalidEncoding);
        }
        let mut writer = Writer::new();
        writer.u8(self.version);
        writer.bytes_u32(&witness)?;
        writer.u32(
            u32::try_from(ordered_external.len()).map_err(|_| WireError::LengthOverflow)?,
        );
        let mut external_bytes = 0usize;
        for external in ordered_external {
            let encoded = external.encode()?;
            external_bytes = external_bytes
                .checked_add(encoded.len())
                .ok_or(WireError::LengthOverflow)?;
            if external_bytes > MAX_EXTERNAL_STATE_BYTES {
                return Err(WireError::TooManyItems);
            }
            writer.bytes_u32(&encoded)?;
        }
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
        let external_count = reader.u32()? as usize;
        if external_count > MAX_EXTERNAL_STATE_WITNESSES {
            return Err(WireError::TooManyItems);
        }
        let mut external_state = Vec::with_capacity(external_count);
        let mut external_bytes = 0usize;
        for _ in 0..external_count {
            let remaining = MAX_EXTERNAL_STATE_BYTES
                .checked_sub(external_bytes)
                .ok_or(WireError::TooManyItems)?;
            let encoded = reader.bytes_limited(remaining)?;
            external_bytes = external_bytes
                .checked_add(encoded.len())
                .ok_or(WireError::LengthOverflow)?;
            external_state.push(ExternalStateWitnessV1::decode(&encoded)?);
        }
        let dependencies = dependencies_from_external_witnesses(&external_state)?;
        if external_state
            .iter()
            .map(|witness| witness.service_id)
            .ne(dependencies.iter().map(|dependency| dependency.service_id))
        {
            return Err(WireError::UnsortedKeys);
        }
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
            external_state,
            actions,
        })
    }
}
'''
replace_once(core, old_input, new_input)

# Remove the old orphan ExternalStateWitnessV1 that lived after RecoveryRecordV1.
replace_once(
    core,
    '''#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalStateWitnessV1 {
    pub service_key: ServiceKeyV1,
    pub state_root: StateRoot,
    pub proof: Vec<Vec<u8>>,
}

''',
    '',
)

replace_once(
    core,
    '''pub struct RuntimeRefineOutputV1 {
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
''',
    '''pub struct RuntimeRefineOutputV1 {
    pub version: u8,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub transition_valid_until: Option<u64>,
    pub external_dependencies: Vec<ExternalStateDependencyV1>,
    pub receipts: Vec<ActionReceiptV1>,
    pub recovery_commitment: StateRoot,
    pub recovery_payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRefineTransitionHeaderV1 {
    pub version: u8,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub transition_valid_until: Option<u64>,
    pub external_dependencies: Vec<ExternalStateDependencyV1>,
    pub recovery_commitment: StateRoot,
}
''',
)

old_constructor = '''    pub fn from_diff_with_validity(
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
'''
new_constructor = '''    pub fn from_diff_with_validity(
        parent_root: StateRoot,
        new_root: StateRoot,
        receipts: Vec<ActionReceiptV1>,
        diff: StateDiffV1,
        transition_valid_until: Option<u64>,
    ) -> Result<Self, WireError> {
        Self::from_diff_with_validity_and_dependencies(
            parent_root,
            new_root,
            receipts,
            diff,
            transition_valid_until,
            Vec::new(),
        )
    }

    pub fn from_diff_with_validity_and_dependencies(
        parent_root: StateRoot,
        new_root: StateRoot,
        receipts: Vec<ActionReceiptV1>,
        diff: StateDiffV1,
        transition_valid_until: Option<u64>,
        external_dependencies: Vec<ExternalStateDependencyV1>,
    ) -> Result<Self, WireError> {
        validate_external_dependencies(&external_dependencies)?;
        let recovery = StateRecoveryV1::new(diff)?;
        let recovery_payload = recovery.encode()?;
        let recovery_commitment = blake2_256(&recovery_payload);
        Ok(Self {
            version: MANAGED_STATE_PROTOCOL_VERSION,
            parent_root,
            new_root,
            transition_valid_until,
            external_dependencies,
            receipts,
            recovery_commitment,
            recovery_payload,
        })
    }
'''
replace_once(core, old_constructor, new_constructor)

replace_once(
    core,
    '''        if self.version != MANAGED_STATE_PROTOCOL_VERSION
            || self.receipts.len() > MAX_RUNTIME_ACTIONS
            || self.recovery_payload.len() > MAX_RECOVERY_BYTES
        {
            return Err(WireError::TooManyItems);
        }
        let mut writer = Writer::new();
''',
    '''        if self.version != MANAGED_STATE_PROTOCOL_VERSION
            || self.receipts.len() > MAX_RUNTIME_ACTIONS
            || self.recovery_payload.len() > MAX_RECOVERY_BYTES
        {
            return Err(WireError::TooManyItems);
        }
        validate_external_dependencies(&self.external_dependencies)?;
        let mut writer = Writer::new();
''',
)

# Insert dependencies into output wire immediately after validity and before recovery commitment.
replace_once(
    core,
    '''            None => writer.u8(0),
        }
        writer.raw(&self.recovery_commitment);
        writer.u32(self.receipts.len() as u32);
''',
    '''            None => writer.u8(0),
        }
        writer.u32(
            u32::try_from(self.external_dependencies.len())
                .map_err(|_| WireError::LengthOverflow)?,
        );
        for dependency in &self.external_dependencies {
            writer.u32(dependency.service_id);
            writer.raw(&dependency.state_root);
        }
        writer.raw(&self.recovery_commitment);
        writer.u32(self.receipts.len() as u32);
''',
)

# Transition header decode: parse dependency list after validity.
replace_once(
    core,
    '''        let recovery_commitment = reader.array::<32>()?;
        let count = reader.u32()? as usize;
''',
    '''        let dependency_count = reader.u32()? as usize;
        if dependency_count > MAX_EXTERNAL_STATE_WITNESSES {
            return Err(WireError::TooManyItems);
        }
        let mut external_dependencies = Vec::with_capacity(dependency_count);
        for _ in 0..dependency_count {
            external_dependencies.push(ExternalStateDependencyV1 {
                service_id: reader.u32()?,
                state_root: reader.array::<32>()?,
            });
        }
        validate_external_dependencies(&external_dependencies)?;
        let recovery_commitment = reader.array::<32>()?;
        let count = reader.u32()? as usize;
''',
)

replace_once(
    core,
    '''            transition_valid_until,
            recovery_commitment,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
''',
    '''            transition_valid_until,
            external_dependencies,
            recovery_commitment,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
''',
)

# Full decode has another recovery_commitment occurrence; parse dependencies there as well.
text = core.read_text()
marker = '''        let recovery_commitment = reader.array::<32>()?;
        let count = reader.u32()? as usize;
'''
if text.count(marker) != 1:
    raise SystemExit(f"{core}: expected one remaining full-decode recovery marker, found {text.count(marker)}")
text = text.replace(
    marker,
    '''        let dependency_count = reader.u32()? as usize;
        if dependency_count > MAX_EXTERNAL_STATE_WITNESSES {
            return Err(WireError::TooManyItems);
        }
        let mut external_dependencies = Vec::with_capacity(dependency_count);
        for _ in 0..dependency_count {
            external_dependencies.push(ExternalStateDependencyV1 {
                service_id: reader.u32()?,
                state_root: reader.array::<32>()?,
            });
        }
        validate_external_dependencies(&external_dependencies)?;
        let recovery_commitment = reader.array::<32>()?;
        let count = reader.u32()? as usize;
''',
    1,
)
core.write_text(text)

replace_once(
    core,
    '''            transition_valid_until,
            receipts,
            recovery_commitment,
            recovery_payload,
        };
''',
    '''            transition_valid_until,
            external_dependencies,
            receipts,
            recovery_commitment,
            recovery_payload,
        };
''',
)

# Add read-only external state access to ExecutionContext while keeping old constructors source-compatible.
replace_once(
    core,
    '''pub struct ExecutionContext<'a> {
    state: &'a mut dyn ManagedStateAccess,
    access_plan: Option<&'a StateAccessPlanV1>,
    sender: Option<[u8; 32]>,
    transition_valid_until: Option<u64>,
}
''',
    '''pub struct ExecutionContext<'a> {
    state: &'a mut dyn ManagedStateAccess,
    external_state: Option<&'a mut dyn ExternalStateAccess>,
    access_plan: Option<&'a StateAccessPlanV1>,
    sender: Option<[u8; 32]>,
    transition_valid_until: Option<u64>,
}
''',
)

replace_once(
    core,
    '''        Self {
            state,
            access_plan: None,
            sender,
            transition_valid_until: None,
        }
''',
    '''        Self {
            state,
            external_state: None,
            access_plan: None,
            sender,
            transition_valid_until: None,
        }
''',
)

replace_once(
    core,
    '''        Self {
            state,
            access_plan: Some(access_plan),
            sender,
            transition_valid_until: None,
        }
    }

    pub fn state(&mut self) -> &mut dyn ManagedStateAccess {
''',
    '''        Self {
            state,
            external_state: None,
            access_plan: Some(access_plan),
            sender,
            transition_valid_until: None,
        }
    }

    pub fn with_access_plan_and_external(
        state: &'a mut dyn ManagedStateAccess,
        external_state: &'a mut dyn ExternalStateAccess,
        sender: Option<[u8; 32]>,
        access_plan: &'a StateAccessPlanV1,
    ) -> Self {
        Self {
            state,
            external_state: Some(external_state),
            access_plan: Some(access_plan),
            sender,
            transition_valid_until: None,
        }
    }

    pub fn state(&mut self) -> &mut dyn ManagedStateAccess {
''',
)

replace_once(
    core,
    '''    pub fn state_view(&mut self) -> Result<StateViewV1, StateAccessError> {
''',
    '''    pub fn external_get(
        &mut self,
        service_id: u32,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StateAccessError> {
        self.external_state
            .as_deref_mut()
            .ok_or(StateAccessError::MissingWitness)?
            .get(service_id, key)
    }

    pub fn state_view(&mut self) -> Result<StateViewV1, StateAccessError> {
''',
)

replace_once(
    core,
    '''pub trait ManagedStateAccess {
''',
    '''pub trait ExternalStateAccess {
    fn get(&mut self, service_id: u32, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError>;
}

pub trait ManagedStateAccess {
''',
)

replace_once(
    core,
    '''    NeedState(Vec<u8>),
    Rejected(u32),
''',
    '''    NeedState(Vec<u8>),
    NeedExternalState { service_id: u32, key: Vec<u8> },
    Rejected(u32),
''',
)

# ---------------- guest ----------------
replace_once(
    guest,
    '''use alloc::vec::Vec;
use service_runtime_core::StateDiffV1;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, ManagedStateAccess,
    RuntimeRefineInputV1, RuntimeRefineOutputV1, ServiceApplication, StateAccessError,
};
''',
    '''use alloc::{collections::BTreeMap, vec::Vec};
use service_runtime_core::StateDiffV1;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, ExternalStateAccess,
    ExternalStateDependencyV1, RuntimeRefineInputV1, RuntimeRefineOutputV1, ServiceApplication,
    StateAccessError,
};
''',
)

replace_once(
    guest,
    '''type RefineTransition = (
    service_runtime_core::StateRoot,
    service_runtime_core::StateRoot,
    Vec<ActionReceiptV1>,
    StateDiffV1,
    Option<u64>,
);
''',
    '''type RefineTransition = (
    service_runtime_core::StateRoot,
    service_runtime_core::StateRoot,
    Vec<ActionReceiptV1>,
    StateDiffV1,
    Option<u64>,
    Vec<ExternalStateDependencyV1>,
);

struct ExternalProofStates {
    states: BTreeMap<u32, (service_runtime_core::StateAccessPlanV1, ProofState)>,
}

impl ExternalProofStates {
    fn from_input(input: &RuntimeRefineInputV1) -> Result<Self, GuestError> {
        let mut states = BTreeMap::new();
        for witness in &input.external_state {
            if states.contains_key(&witness.service_id) {
                return Err(GuestError::InvalidInput);
            }
            let mut state = ProofState::from_witness(
                witness.managed_state.parent_root,
                &witness.managed_state.storage_proof,
            )
            .map_err(|_| GuestError::State)?;
            for key in &witness.managed_state.access_plan.keys {
                state.get(key).map_err(|_| GuestError::State)?;
            }
            states.insert(
                witness.service_id,
                (witness.managed_state.access_plan.clone(), state),
            );
        }
        Ok(Self { states })
    }
}

impl ExternalStateAccess for ExternalProofStates {
    fn get(&mut self, service_id: u32, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        let (plan, state) = self
            .states
            .get_mut(&service_id)
            .ok_or(StateAccessError::NeedExternalState {
                service_id,
                key: key.to_vec(),
            })?;
        if plan.keys.binary_search_by(|candidate| candidate.as_slice().cmp(key)).is_err() {
            return Err(StateAccessError::NeedExternalState {
                service_id,
                key: key.to_vec(),
            });
        }
        state.get(key)
    }
}
''',
)

# Update both public refine constructors to carry dependencies.
replace_once(
    guest,
    '''    let (parent_root, new_root, receipts, diff, transition_valid_until) =
        refine_internal(application, input)?;
    RuntimeRefineOutputV1::from_diff_with_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
    )
''',
    '''    let (
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
        external_dependencies,
    ) = refine_internal(application, input)?;
    RuntimeRefineOutputV1::from_diff_with_validity_and_dependencies(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
        external_dependencies,
    )
''',
)

# In non-observer refine, create external proof states and pass them into context.
replace_once(
    guest,
    '''    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(input.actions.len());
''',
    '''    let parent_root = state.parent_root();
    let mut external_state = ExternalProofStates::from_input(input)?;
    let external_dependencies = input
        .external_dependencies()
        .map_err(|_| GuestError::InvalidInput)?;
    let mut receipts = Vec::with_capacity(input.actions.len());
''',
)

replace_once(
    guest,
    '''            let mut context = ExecutionContext::with_access_plan(
                &mut state,
                None,
                &input.managed_state.access_plan,
            );
''',
    '''            let mut context = ExecutionContext::with_access_plan_and_external(
                &mut state,
                &mut external_state,
                None,
                &input.managed_state.access_plan,
            );
''',
)

replace_once(
    guest,
    '''            Err(StateAccessError::NeedState(_)) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::State);
            }
''',
    '''            Err(StateAccessError::NeedState(_)
            | StateAccessError::NeedExternalState { .. }) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::State);
            }
''',
)

# First tuple return in refine_internal.
replace_once(
    guest,
    '''        diff,
        transition_valid_until,
    ))
}

pub fn refine_owned_with_observer''',
    '''        diff,
        transition_valid_until,
        external_dependencies,
    ))
}

pub fn refine_owned_with_observer''',
)

# Observer constructor.
replace_once(
    guest,
    '''    let (parent_root, new_root, receipts, diff, transition_valid_until) =
        refine_internal_owned_with_observer(application, input, observer)?;
    RuntimeRefineOutputV1::from_diff_with_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
    )
''',
    '''    let (
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
        external_dependencies,
    ) = refine_internal_owned_with_observer(application, input, observer)?;
    RuntimeRefineOutputV1::from_diff_with_validity_and_dependencies(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
        external_dependencies,
    )
''',
)

# Observer path input is owned; derive deps and build external proof states before moving fields.
replace_once(
    guest,
    '''    let mut state = ProofState::from_witness_owned_with_observer(
        input.managed_state.parent_root,
        input.managed_state.storage_proof,
        |stage| observer.stage(stage),
    )
''',
    '''    let external_dependencies = input
        .external_dependencies()
        .map_err(|_| GuestError::InvalidInput)?;
    let mut external_state = ExternalProofStates::from_input(&input)?;
    let managed_access_plan = input.managed_state.access_plan.clone();
    let actions = input.actions;
    let mut state = ProofState::from_witness_owned_with_observer(
        input.managed_state.parent_root,
        input.managed_state.storage_proof,
        |stage| observer.stage(stage),
    )
''',
)

replace_once(
    guest,
    '''    for key in &input.managed_state.access_plan.keys {
        state.get(key).map_err(|_| GuestError::State)?;
    }
    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(input.actions.len());
    let mut transition_valid_until = None;
    for action in &input.actions {
''',
    '''    for key in &managed_access_plan.keys {
        state.get(key).map_err(|_| GuestError::State)?;
    }
    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(actions.len());
    let mut transition_valid_until = None;
    for action in &actions {
''',
)

# There is one remaining observer ExecutionContext::with_access_plan.
replace_once(
    guest,
    '''            let mut context = ExecutionContext::with_access_plan(
                &mut state,
                None,
                &input.managed_state.access_plan,
            );
''',
    '''            let mut context = ExecutionContext::with_access_plan_and_external(
                &mut state,
                &mut external_state,
                None,
                &managed_access_plan,
            );
''',
)

# Observer path may not have explicit NeedState arm. Insert before ApplicationFailed.
replace_once(
    guest,
    '''            Err(StateAccessError::ApplicationFailed(error_code)) => {
''',
    '''            Err(StateAccessError::NeedState(_)
            | StateAccessError::NeedExternalState { .. }) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::State);
            }
            Err(StateAccessError::ApplicationFailed(error_code)) => {
''',
)

# Observer tuple return at end.
replace_once(
    guest,
    '''        diff,
        transition_valid_until,
    ))
}

fn merge_validity''',
    '''        diff,
        transition_valid_until,
        external_dependencies,
    ))
}

fn merge_validity''',
)

# ---------------- codegen accumulate ----------------
replace_once(
    codegen,
    '''    fn minijam_storage_read(key: *const u8, key_size: usize, output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    fn minijam_storage_write(key: *const u8, key_size: usize, value: *const u8, value_size: usize) -> u32;
''',
    '''    fn minijam_storage_read(key: *const u8, key_size: usize, output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    fn minijam_service_storage_read(service_id: u32, key: *const u8, key_size: usize, output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    fn minijam_storage_write(key: *const u8, key_size: usize, value: *const u8, value_size: usize) -> u32;
''',
)

replace_once(
    codegen,
    '''        if header.parent_root != current {{ continue; }}
        if header.transition_valid_until.is_some_and(|valid_until| authoritative_tick > valid_until) {{ continue; }}
        current = header.new_root;
''',
    '''        if header.parent_root != current {{ continue; }}
        if header.transition_valid_until.is_some_and(|valid_until| authoritative_tick > valid_until) {{ continue; }}
        if !external_dependencies_match(&header.external_dependencies) {{ continue; }}
        current = header.new_root;
''',
)

replace_once(
    codegen,
    '''fn read_current_commitment() -> Result<StateRoot, ()> {{
''',
    '''fn external_dependencies_match(
    dependencies: &[service_runtime_core::ExternalStateDependencyV1],
) -> bool {{
    let key = MANAGED_STATE_COMMITMENT_KEY_V1;
    for dependency in dependencies {{
        let mut bytes = [0u8; 34];
        let mut size = 0usize;
        let status = unsafe {{
            minijam_service_storage_read(
                dependency.service_id,
                key.as_ptr(),
                key.len(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut size,
            )
        }};
        let current = match status {{
            1 => service_runtime_core::EMPTY_STATE_ROOT_V1,
            0 if size == bytes.len() => match ManagedStateCommitmentV1::decode(&bytes) {{
                Ok(commitment) => commitment.root,
                Err(_) => return false,
            }},
            _ => return false,
        }};
        if current != dependency.state_root {{ return false; }}
    }}
    true
}}

fn read_current_commitment() -> Result<StateRoot, ()> {{
''',
)

# Add empty external_state to every remaining RuntimeRefineInputV1 literal in Rust sources.
for path in ROOT.rglob('*.rs'):
    if path == core:
        continue
    text = path.read_text()
    if 'RuntimeRefineInputV1 {' not in text:
        continue
    # Conservative insertion only when managed_state is immediately followed later by actions
    # within a literal and external_state is absent in that slice.
    pattern = re.compile(r'(RuntimeRefineInputV1\s*\{.*?managed_state:\s*[^\n]+,\n)(\s*)(actions:)', re.S)
    def add_external(match):
        segment = match.group(0)
        if 'external_state:' in segment:
            return segment
        return match.group(1) + match.group(2) + 'external_state: Vec::new(),\n' + match.group(2) + match.group(3)
    updated = pattern.sub(add_external, text)
    if updated != text:
        path.write_text(updated)

print('external-state-v1 patch applied')
