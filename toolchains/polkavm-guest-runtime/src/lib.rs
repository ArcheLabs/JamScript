#![no_std]
#![allow(static_mut_refs)]
#![allow(unexpected_cfgs)]

extern crate alloc;
#[cfg(not(target_env = "polkavm"))]
extern crate std;

use alloc::vec::Vec;
use jamscript_runtime_core::{
    decode_signed_action_v1, decode_signed_action_v2, nonce_key, ownership_nonce_key,
    verify_signed_action_v1, verify_signed_action_v2,
};
use service_runtime_core::{
    BackendMetadataV1, RuntimeRefineInputV1, RuntimeRefineOutputV1, ScriptActionResultV1,
    ServiceApplication, ServiceKeyV1, StateAccessError, MAX_SCRIPT_ACTION_RESULT_BYTES,
};
#[cfg(target_env = "polkavm")]
use service_runtime_core::{ManagedStateCommitmentV1, StateRoot, MANAGED_STATE_COMMITMENT_KEY_V1};

const DESCRIPTOR_VERSION: u32 = 1;
const AUTH_PUBLIC: u8 = 0;
const AUTH_WALLET: u8 = 1;
const AUTH_OWNERSHIP: u8 = 2;
const MAX_DESCRIPTOR_ACTIONS: u32 = 1024;
const MAX_DESCRIPTOR_NAMESPACES: u32 = 1024;
const MAX_DESCRIPTOR_NAMESPACE_BYTES: usize = 4096;

#[cfg(not(target_env = "polkavm"))]
#[global_allocator]
static HOST_ALLOCATOR: std::alloc::System = std::alloc::System;

#[repr(C)]
#[derive(Clone, Copy)]
struct JamScriptActionDescriptorV1 {
    selector: [u8; 8],
    auth_kind: u8,
    reserved: [u8; 7],
    entry: Option<
        unsafe extern "C" fn(
            payload: *const u8,
            payload_len: usize,
            auth_context: *const u8,
            auth_context_len: usize,
            state_view: *const u8,
            state_view_len: usize,
            output: *mut *const u8,
            output_len: *mut usize,
        ),
    >,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct JamScriptNamespaceDescriptorV1 {
    bytes: *const u8,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct JamScriptServiceDescriptorV1 {
    version: u32,
    action_count: u32,
    actions: *const JamScriptActionDescriptorV1,
    namespace_count: u32,
    namespaces: *const JamScriptNamespaceDescriptorV1,
    service_key: [u8; 32],
    service_instance_id: [u8; 32],
    management_policy_kind: u8,
    reserved: [u8; 7],
    management_account: [u8; 32],
    init: Option<unsafe extern "C" fn()>,
}

#[repr(C)]
pub struct RefineOutput {
    pub data: *const u8,
    pub size: usize,
}

unsafe extern "C" {
    static jamscript_service_descriptor_v1: JamScriptServiceDescriptorV1;

    fn minijam_payload(output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
}

#[cfg(target_env = "polkavm")]
unsafe extern "C" {
    fn minijam_result_count() -> usize;
    fn minijam_result(
        index: usize,
        output: *mut u8,
        capacity: usize,
        output_size: *mut usize,
    ) -> u32;
    fn minijam_storage_read(
        key: *const u8,
        key_size: usize,
        output: *mut u8,
        capacity: usize,
        output_size: *mut usize,
    ) -> u32;
    fn minijam_service_storage_read(
        service_id: u32,
        key: *const u8,
        key_size: usize,
        output: *mut u8,
        capacity: usize,
        output_size: *mut usize,
    ) -> u32;
    fn minijam_storage_write(
        key: *const u8,
        key_size: usize,
        value: *const u8,
        value_size: usize,
    ) -> u32;
}

static mut INPUT: [u8; 1_048_576] = [0; 1_048_576];
#[cfg(target_env = "polkavm")]
static mut RESULT: [u8; 2_097_152] = [0; 2_097_152];
static mut OUTPUT: [u8; 2_097_152] = [0; 2_097_152];

fn descriptor() -> Result<&'static JamScriptServiceDescriptorV1, service_runtime_guest::GuestError>
{
    let descriptor = unsafe { &jamscript_service_descriptor_v1 };
    if descriptor.version != DESCRIPTOR_VERSION
        || descriptor.action_count == 0
        || descriptor.action_count > MAX_DESCRIPTOR_ACTIONS
        || descriptor.actions.is_null()
        || descriptor.namespace_count > MAX_DESCRIPTOR_NAMESPACES
        || (descriptor.namespace_count != 0 && descriptor.namespaces.is_null())
    {
        return Err(service_runtime_guest::GuestError::InvalidInput);
    }
    for index in 0..descriptor.action_count as usize {
        let action = unsafe { &*descriptor.actions.add(index) };
        if action.auth_kind > AUTH_OWNERSHIP || action.entry.is_none() {
            return Err(service_runtime_guest::GuestError::InvalidInput);
        }
    }
    for index in 0..descriptor.namespace_count as usize {
        let namespace = unsafe { &*descriptor.namespaces.add(index) };
        if namespace.bytes.is_null() || namespace.len > MAX_DESCRIPTOR_NAMESPACE_BYTES {
            return Err(service_runtime_guest::GuestError::InvalidInput);
        }
    }
    Ok(descriptor)
}

fn service_key(descriptor: &JamScriptServiceDescriptorV1) -> ServiceKeyV1 {
    ServiceKeyV1::new(descriptor.service_key)
}

fn find_action(
    descriptor: &JamScriptServiceDescriptorV1,
    selector: [u8; 8],
) -> Result<&'static JamScriptActionDescriptorV1, StateAccessError> {
    for index in 0..descriptor.action_count as usize {
        let action = unsafe { &*descriptor.actions.add(index) };
        if action.selector == selector {
            return Ok(action);
        }
    }
    Err(StateAccessError::Rejected(
        jamscript_runtime_core::RuntimeError::UnknownAction.code(),
    ))
}

fn signed_selector(raw_action: &[u8]) -> Result<[u8; 8], StateAccessError> {
    raw_action
        .get(65..73)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            StateAccessError::Rejected(jamscript_runtime_core::RuntimeError::InvalidEnvelope.code())
        })
}

struct DescriptorApplication;

impl ServiceApplication for DescriptorApplication {
    type Error = StateAccessError;

    fn execute(
        &self,
        context: &mut service_runtime_core::ExecutionContext<'_>,
        raw_action: &[u8],
    ) -> Result<(), Self::Error> {
        let descriptor = descriptor().map_err(|_| StateAccessError::Backend)?;
        let public_action = descriptor.action_count == 1
            && unsafe { (*descriptor.actions).auth_kind == AUTH_PUBLIC };
        let selector = if public_action {
            unsafe { (*descriptor.actions).selector }
        } else {
            signed_selector(raw_action)?
        };
        let action = find_action(descriptor, selector)?;
        let (payload, auth_context) = authenticate(context, descriptor, action, raw_action)?;
        execute_scriptc(context, action, payload, &auth_context)
    }
}

fn authenticate<'a>(
    context: &mut service_runtime_core::ExecutionContext<'_>,
    descriptor: &JamScriptServiceDescriptorV1,
    action: &JamScriptActionDescriptorV1,
    raw_action: &'a [u8],
) -> Result<(&'a [u8], Vec<u8>), StateAccessError> {
    match action.auth_kind {
        AUTH_PUBLIC => {
            if descriptor.action_count != 1 {
                return Err(StateAccessError::Rejected(
                    jamscript_runtime_core::RuntimeError::UnknownAction.code(),
                ));
            }
            Ok((raw_action, Vec::new()))
        }
        AUTH_WALLET => {
            let signed = decode_signed_action_v1(raw_action)
                .map_err(|error| StateAccessError::Rejected(error.code()))?;
            let verified = verify_signed_action_v1(
                signed,
                context.network_domain(),
                service_key(descriptor),
                action.selector,
            )
            .map_err(|error| StateAccessError::Rejected(error.code()))?;
            let nonce_key = nonce_key(&verified.sender);
            let expected_nonce = read_nonce(context, &nonce_key)?;
            if verified.nonce != expected_nonce {
                return Err(StateAccessError::Rejected(
                    jamscript_runtime_core::RuntimeError::NonceMismatch.code(),
                ));
            }
            context.constrain_valid_until(verified.valid_until);
            let next_nonce = expected_nonce
                .checked_add(1)
                .ok_or(StateAccessError::Backend)?;
            context.state().set(&nonce_key, &next_nonce.to_le_bytes())?;
            Ok((verified.payload, verified.sender.to_vec()))
        }
        AUTH_OWNERSHIP => {
            let signed = decode_signed_action_v2(raw_action)
                .map_err(|error| StateAccessError::Rejected(error.code()))?;
            if signed.act_as.is_some() {
                return Err(StateAccessError::Rejected(
                    jamscript_runtime_core::RuntimeError::ActAsUnsupported.code(),
                ));
            }
            let nonce_key = ownership_nonce_key(&signed.controller)
                .map_err(|error| StateAccessError::Rejected(error.code()))?;
            let expected_nonce = read_nonce(context, &nonce_key)?;
            let verified = verify_signed_action_v2(
                signed,
                context.network_domain(),
                service_key(descriptor),
                action.selector,
                Some(expected_nonce),
            )
            .map_err(|error| StateAccessError::Rejected(error.code()))?;
            context.set_ownership(verified.owner.clone(), verified.controller.clone());
            context.constrain_valid_until(verified.valid_until);
            let next_nonce = expected_nonce
                .checked_add(1)
                .ok_or(StateAccessError::Backend)?;
            context.state().set(&nonce_key, &next_nonce.to_le_bytes())?;
            let owner = verified
                .owner
                .encode()
                .map_err(|_| StateAccessError::Backend)?;
            let controller = verified
                .controller
                .encode()
                .map_err(|_| StateAccessError::Backend)?;
            if owner.len() > u16::MAX as usize || controller.len() > u16::MAX as usize {
                return Err(StateAccessError::Backend);
            }
            let mut auth_context = Vec::with_capacity(1 + 2 + owner.len() + 2 + controller.len());
            auth_context.push(1);
            auth_context.extend_from_slice(&(owner.len() as u16).to_le_bytes());
            auth_context.extend_from_slice(&owner);
            auth_context.extend_from_slice(&(controller.len() as u16).to_le_bytes());
            auth_context.extend_from_slice(&controller);
            Ok((verified.payload, auth_context))
        }
        _ => Err(StateAccessError::Backend),
    }
}

fn read_nonce(
    context: &mut service_runtime_core::ExecutionContext<'_>,
    key: &[u8],
) -> Result<u64, StateAccessError> {
    match context.state().get(key)? {
        None => Ok(0),
        Some(bytes) if bytes.len() == 8 => Ok(u64::from_le_bytes(
            bytes.try_into().map_err(|_| StateAccessError::Backend)?,
        )),
        Some(_) => Err(StateAccessError::Backend),
    }
}

fn application_key_allowed(descriptor: &JamScriptServiceDescriptorV1, key: &[u8]) -> bool {
    if key.len() < 3 || key[0] != service_runtime_core::APPLICATION_KEY_CLASS_V1 {
        return false;
    }
    let namespace_len = u16::from_le_bytes([key[1], key[2]]) as usize;
    let Some(namespace) = key.get(3..3usize.saturating_add(namespace_len)) else {
        return false;
    };
    for index in 0..descriptor.namespace_count as usize {
        let entry = unsafe { &*descriptor.namespaces.add(index) };
        if entry.len == namespace.len()
            && !entry.bytes.is_null()
            && unsafe { core::slice::from_raw_parts(entry.bytes, entry.len) } == namespace
        {
            return true;
        }
    }
    false
}

fn apply_script_result(
    context: &mut service_runtime_core::ExecutionContext<'_>,
    descriptor: &JamScriptServiceDescriptorV1,
    result: ScriptActionResultV1,
) -> Result<(), StateAccessError> {
    match result {
        ScriptActionResultV1::Applied(diff) => {
            for change in diff.changes {
                if !application_key_allowed(descriptor, &change.key) {
                    return Err(StateAccessError::ReservedKey);
                }
                match change.value {
                    Some(value) => context.state().set(&change.key, &value)?,
                    None => context.state().delete(&change.key)?,
                }
            }
            Ok(())
        }
        ScriptActionResultV1::Abort(code) => Err(StateAccessError::ApplicationFailed(code)),
        ScriptActionResultV1::NeedState(key) => Err(StateAccessError::NeedState(key)),
        ScriptActionResultV1::Fatal(code) => Err(StateAccessError::ApplicationFailed(code)),
    }
}

fn execute_scriptc(
    context: &mut service_runtime_core::ExecutionContext<'_>,
    action: &JamScriptActionDescriptorV1,
    payload: &[u8],
    auth_context: &[u8],
) -> Result<(), StateAccessError> {
    let entry = action.entry.ok_or(StateAccessError::Backend)?;
    let descriptor = descriptor().map_err(|_| StateAccessError::Backend)?;
    context.begin_transaction()?;
    let result = (|| {
        let state_view = context
            .state_view()?
            .encode()
            .map_err(|_| StateAccessError::Backend)?;
        let mut output = core::ptr::null();
        let mut output_len = 0usize;
        if let Some(init) = descriptor.init {
            unsafe { init() };
        }
        unsafe {
            entry(
                payload.as_ptr(),
                payload.len(),
                auth_context.as_ptr(),
                auth_context.len(),
                state_view.as_ptr(),
                state_view.len(),
                &mut output,
                &mut output_len,
            );
        }
        if output.is_null() && output_len != 0 || output_len > MAX_SCRIPT_ACTION_RESULT_BYTES {
            return Err(StateAccessError::ApplicationFailed(0x8000_0002));
        }
        let bytes = if output_len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(output, output_len) }
        };
        let result = ScriptActionResultV1::decode(bytes)
            .map_err(|_| StateAccessError::ApplicationFailed(0x8000_0002))?;
        apply_script_result(context, descriptor, result)
    })();
    match result {
        Ok(()) => context.commit_transaction(),
        Err(error) => {
            context.rollback_transaction()?;
            Err(error)
        }
    }
}

fn run_refine() -> Result<RuntimeRefineOutputV1, service_runtime_guest::GuestError> {
    service_runtime_guest::guest_support::reset_runtime();
    let mut input_size = 0usize;
    let status = unsafe { minijam_payload(INPUT.as_mut_ptr(), INPUT.len(), &mut input_size) };
    if status != 0 {
        return Err(service_runtime_guest::GuestError::InvalidInput);
    }
    let input = unsafe { core::slice::from_raw_parts(INPUT.as_ptr(), input_size) };
    let runtime_input = RuntimeRefineInputV1::decode(input)
        .map_err(|_| service_runtime_guest::GuestError::InvalidInput)?;
    service_runtime_guest::refine_owned(&DescriptorApplication, runtime_input)
}

fn run_plan() -> Result<(), service_runtime_guest::GuestError> {
    service_runtime_guest::guest_support::reset_runtime();
    let mut input_size = 0usize;
    let status = unsafe { minijam_payload(INPUT.as_mut_ptr(), INPUT.len(), &mut input_size) };
    if status != 0 {
        return Err(service_runtime_guest::GuestError::InvalidInput);
    }
    let input = unsafe { core::slice::from_raw_parts(INPUT.as_ptr(), input_size) };
    let runtime_input = RuntimeRefineInputV1::decode(input)
        .map_err(|_| service_runtime_guest::GuestError::InvalidInput)?;
    service_runtime_guest::plan_owned_with_external(&DescriptorApplication, runtime_input)
}

fn output_for(planning: bool) -> RefineOutput {
    if planning {
        return match run_plan() {
            Ok(()) => planner_done_output(),
            Err(service_runtime_guest::GuestError::NeedState(key)) => {
                let encoded = match service_runtime_core::encode_planner_need_state(&key) {
                    Ok(value) => value,
                    Err(_) => return error_output(2),
                };
                write_output(&encoded)
            }
            Err(service_runtime_guest::GuestError::NeedExternalState { service_id, key }) => {
                let encoded = match service_runtime_core::encode_planner_need_external_state(
                    service_id, &key,
                ) {
                    Ok(value) => value,
                    Err(_) => return error_output(2),
                };
                write_output(&encoded)
            }
            Err(_) => error_output(2),
        };
    }
    let output = match run_refine() {
        Ok(output) => output,
        Err(service_runtime_guest::GuestError::NeedState(key)) => {
            let encoded = match service_runtime_core::encode_planner_need_state(&key) {
                Ok(value) => value,
                Err(_) => return error_output(2),
            };
            return write_output(&encoded);
        }
        Err(service_runtime_guest::GuestError::NeedExternalState { service_id, key }) => {
            let encoded =
                match service_runtime_core::encode_planner_need_external_state(service_id, &key) {
                    Ok(value) => value,
                    Err(_) => return error_output(2),
                };
            return write_output(&encoded);
        }
        Err(_) => return error_output(2),
    };
    let encoded = match output.encode() {
        Ok(value) => value,
        Err(_) => return error_output(2),
    };
    write_output(&encoded)
}

fn write_output(bytes: &[u8]) -> RefineOutput {
    if bytes.len() > 2_097_152 {
        return error_output(14);
    }
    unsafe { OUTPUT[..bytes.len()].copy_from_slice(bytes) };
    RefineOutput {
        data: unsafe { OUTPUT.as_ptr() },
        size: bytes.len(),
    }
}

#[no_mangle]
pub extern "C" fn minijam_refine() -> RefineOutput {
    output_for(false)
}

#[no_mangle]
pub extern "C" fn jamscript_plan_v1() -> RefineOutput {
    output_for(true)
}

#[no_mangle]
pub extern "C" fn jamscript_backend_metadata_v1() -> RefineOutput {
    let descriptor = match descriptor() {
        Ok(value) => value,
        Err(_) => return error_output(2),
    };
    let encoded = BackendMetadataV1 {
        service_key: service_key(descriptor),
        abi_version: 1,
        planner_version: service_runtime_core::BACKEND_PLANNER_VERSION,
        managed_state_version: service_runtime_core::BACKEND_MANAGED_STATE_VERSION,
    }
    .encode();
    write_output(&encoded)
}

#[cfg(target_env = "polkavm")]
#[no_mangle]
pub extern "C" fn minijam_accumulate() {
    let init_pointer: usize;
    let init_size: usize;
    unsafe {
        core::arch::asm!(
            "mv t0, a0",
            "mv t1, a1",
            lateout("t0") init_pointer,
            lateout("t1") init_size,
            options(nomem, nostack, preserves_flags),
        );
    }
    let init_input = unsafe { core::slice::from_raw_parts(init_pointer as *const u8, init_size) };
    let Ok((authoritative_tick, _service_id, _items_count)) =
        decode_accumulate_init_input(init_input)
    else {
        return;
    };
    let mut current =
        read_current_commitment().unwrap_or(service_runtime_core::EMPTY_STATE_ROOT_V1);
    let mut advanced = false;
    let count = unsafe { minijam_result_count() };
    for index in 0..count {
        let mut size = 0usize;
        if unsafe { minijam_result(index, RESULT.as_mut_ptr(), RESULT.len(), &mut size) } != 0 {
            continue;
        }
        let refined = unsafe { core::slice::from_raw_parts(RESULT.as_ptr(), size) };
        let Ok(header) = RuntimeRefineOutputV1::decode_transition_header(refined) else {
            continue;
        };
        if header.parent_root != current
            || header
                .transition_valid_until
                .is_some_and(|valid_until| authoritative_tick > valid_until)
        {
            continue;
        }
        let mut dependencies_valid = true;
        for dependency in &header.external_dependencies {
            let Ok(canonical) = read_service_commitment(dependency.service_id) else {
                dependencies_valid = false;
                break;
            };
            if canonical != dependency.state_root {
                dependencies_valid = false;
                break;
            }
        }
        if !dependencies_valid {
            continue;
        }
        current = header.new_root;
        advanced = true;
    }
    if advanced {
        let commitment = ManagedStateCommitmentV1::new(current).encode();
        let key = MANAGED_STATE_COMMITMENT_KEY_V1;
        let _ = unsafe {
            minijam_storage_write(
                key.as_ptr(),
                key.len(),
                commitment.as_ptr(),
                commitment.len(),
            )
        };
    }
}

#[cfg(not(target_env = "polkavm"))]
#[no_mangle]
pub extern "C" fn minijam_accumulate() {}

#[cfg(target_env = "polkavm")]
fn read_current_commitment() -> Result<StateRoot, ()> {
    let key = MANAGED_STATE_COMMITMENT_KEY_V1;
    let mut bytes = [0u8; 34];
    let mut size = 0usize;
    let status = unsafe {
        minijam_storage_read(
            key.as_ptr(),
            key.len(),
            bytes.as_mut_ptr(),
            bytes.len(),
            &mut size,
        )
    };
    match status {
        1 => Ok(service_runtime_core::EMPTY_STATE_ROOT_V1),
        0 if size == bytes.len() => ManagedStateCommitmentV1::decode(&bytes)
            .map(|commitment| commitment.root)
            .map_err(|_| ()),
        _ => Err(()),
    }
}

#[cfg(target_env = "polkavm")]
fn read_service_commitment(service_id: u32) -> Result<StateRoot, ()> {
    let key = MANAGED_STATE_COMMITMENT_KEY_V1;
    let mut bytes = [0u8; 34];
    let mut size = 0usize;
    let status = unsafe {
        minijam_service_storage_read(
            service_id,
            key.as_ptr(),
            key.len(),
            bytes.as_mut_ptr(),
            bytes.len(),
            &mut size,
        )
    };
    match status {
        0 if size == bytes.len() => ManagedStateCommitmentV1::decode(&bytes)
            .map(|commitment| commitment.root)
            .map_err(|_| ()),
        _ => Err(()),
    }
}

#[cfg(target_env = "polkavm")]
fn decode_accumulate_init_input(input: &[u8]) -> Result<(u64, u64, u64), ()> {
    let mut offset = 0usize;
    let tick = read_fnencode(input, &mut offset)?;
    let service_id = read_fnencode(input, &mut offset)?;
    let items_count = read_fnencode(input, &mut offset)?;
    if offset != input.len() {
        return Err(());
    }
    Ok((tick, service_id, items_count))
}

#[cfg(target_env = "polkavm")]
fn read_fnencode(input: &[u8], offset: &mut usize) -> Result<u64, ()> {
    let first = *input.get(*offset).ok_or(())?;
    *offset += 1;
    if first < 0x80 {
        return Ok(first as u64);
    }
    let mut length = 0usize;
    while length < 8 && (first & (0x80u8 >> length)) != 0 {
        length += 1;
    }
    if length == 0 || length > 7 || input.len().saturating_sub(*offset) < length {
        return Err(());
    }
    let mut low = 0u64;
    for index in 0..length {
        low |= (*input.get(*offset + index).ok_or(())? as u64) << (8 * index);
    }
    *offset += length;
    Ok(((first as u64 & (0x7f >> length)) << (8 * length)) | low)
}

fn error_output(code: u32) -> RefineOutput {
    unsafe { OUTPUT[..4].copy_from_slice(&code.to_le_bytes()) };
    RefineOutput {
        data: unsafe { OUTPUT.as_ptr() },
        size: 4,
    }
}

fn planner_done_output() -> RefineOutput {
    unsafe { OUTPUT[0] = 0 };
    RefineOutput {
        data: unsafe { OUTPUT.as_ptr() },
        size: 1,
    }
}
