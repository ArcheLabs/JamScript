#![cfg_attr(target_env = "polkavm", no_std)]
#![allow(static_mut_refs)]
#![allow(unexpected_cfgs)]

extern crate alloc;

use jamscript_ownership_control_service::{
    OwnershipControlService, OWNERSHIP_CONTROL_SERVICE_KEY_V1,
};
use service_runtime_core::{
    BackendMetadataV1, RuntimeRefineInputV1,
};
#[cfg(target_env = "polkavm")]
use service_runtime_core::{
    ManagedStateCommitmentV1, RuntimeRefineOutputV1, StateRoot, MANAGED_STATE_COMMITMENT_KEY_V1,
};

#[repr(C)]
pub struct RefineOutput {
    pub data: *const u8,
    pub size: usize,
}

extern "C" {
    fn minijam_payload(output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    #[cfg(target_env = "polkavm")]
    fn minijam_result_count() -> usize;
    #[cfg(target_env = "polkavm")]
    fn minijam_result(
        index: usize,
        output: *mut u8,
        capacity: usize,
        output_size: *mut usize,
    ) -> u32;
    #[cfg(target_env = "polkavm")]
    fn minijam_storage_read(
        key: *const u8,
        key_size: usize,
        output: *mut u8,
        capacity: usize,
        output_size: *mut usize,
    ) -> u32;
    #[cfg(target_env = "polkavm")]
    fn minijam_service_storage_read(
        service_id: u32,
        key: *const u8,
        key_size: usize,
        output: *mut u8,
        capacity: usize,
        output_size: *mut usize,
    ) -> u32;
    #[cfg(target_env = "polkavm")]
    fn minijam_storage_write(
        key: *const u8,
        key_size: usize,
        value: *const u8,
        value_size: usize,
    ) -> u32;
}

static mut INPUT: [u8; 1_048_576] = [0; 1_048_576];
static mut RESULT: [u8; 2_097_152] = [0; 2_097_152];
static mut OUTPUT: [u8; 2_097_152] = [0; 2_097_152];

fn output_bytes(bytes: &[u8]) -> RefineOutput {
    if bytes.len() > 2_097_152 {
        return RefineOutput {
            data: core::ptr::null(),
            size: 0,
        };
    }
    unsafe { OUTPUT[..bytes.len()].copy_from_slice(bytes) };
    RefineOutput {
        data: unsafe { OUTPUT.as_ptr() },
        size: bytes.len(),
    }
}

fn error_output(code: u32) -> RefineOutput {
    output_bytes(&code.to_le_bytes())
}

fn load_input() -> Result<RuntimeRefineInputV1, service_runtime_guest::GuestError> {
    let mut size = 0usize;
    let status = unsafe { minijam_payload(INPUT.as_mut_ptr(), INPUT.len(), &mut size) };
    if status != 0 {
        return Err(service_runtime_guest::GuestError::InvalidInput);
    }
    RuntimeRefineInputV1::decode(unsafe { &INPUT[..size] })
        .map_err(|_| service_runtime_guest::GuestError::InvalidInput)
}

fn plan_output(result: Result<(), service_runtime_guest::GuestError>) -> RefineOutput {
    match result {
        Ok(()) => output_bytes(&[0]),
        Err(service_runtime_guest::GuestError::NeedState(key)) => {
            match service_runtime_core::encode_planner_need_state(&key) {
                Ok(bytes) => output_bytes(&bytes),
                Err(_) => error_output(2),
            }
        }
        Err(service_runtime_guest::GuestError::NeedExternalState { service_id, key }) => {
            match service_runtime_core::encode_planner_need_external_state(service_id, &key) {
                Ok(bytes) => output_bytes(&bytes),
                Err(_) => error_output(2),
            }
        }
        Err(_) => error_output(2),
    }
}

#[no_mangle]
pub extern "C" fn jamscript_backend_metadata_v1() -> RefineOutput {
    let metadata = BackendMetadataV1 {
        service_key: OWNERSHIP_CONTROL_SERVICE_KEY_V1,
        abi_version: 1,
        planner_version: service_runtime_core::BACKEND_PLANNER_VERSION,
        managed_state_version: service_runtime_core::BACKEND_MANAGED_STATE_VERSION,
    };
    output_bytes(&metadata.encode())
}

#[no_mangle]
pub extern "C" fn jamscript_plan_v1() -> RefineOutput {
    service_runtime_guest::guest_support::reset_runtime();
    let input = match load_input() {
        Ok(input) => input,
        Err(_) => return error_output(1),
    };
    plan_output(service_runtime_guest::plan_owned_with_external(
        &OwnershipControlService,
        input,
    ))
}

#[no_mangle]
pub extern "C" fn minijam_refine() -> RefineOutput {
    service_runtime_guest::guest_support::reset_runtime();
    let input = match load_input() {
        Ok(input) => input,
        Err(_) => return error_output(1),
    };
    let output = match service_runtime_guest::refine_owned(&OwnershipControlService, input) {
        Ok(output) => output,
        Err(_) => return error_output(2),
    };
    match output.encode() {
        Ok(bytes) => output_bytes(&bytes),
        Err(_) => error_output(2),
    }
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
    let Ok((authoritative_tick, _service_id, _items_count)) = decode_accumulate_init_input(init_input)
    else {
        return;
    };
    let mut current = read_current_commitment().unwrap_or(service_runtime_core::EMPTY_STATE_ROOT_V1);
    let mut advanced = false;
    for index in 0..unsafe { minijam_result_count() } {
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
            let Ok(root) = read_service_commitment(dependency.service_id) else {
                dependencies_valid = false;
                break;
            };
            if root != dependency.state_root {
                dependencies_valid = false;
                break;
            }
        }
        if dependencies_valid {
            current = header.new_root;
            advanced = true;
        }
    }
    if advanced {
        let commitment = ManagedStateCommitmentV1::new(current).encode();
        unsafe {
            let _ = minijam_storage_write(
                MANAGED_STATE_COMMITMENT_KEY_V1.as_ptr(),
                MANAGED_STATE_COMMITMENT_KEY_V1.len(),
                commitment.as_ptr(),
                commitment.len(),
            );
        }
    }
}

#[cfg(not(target_env = "polkavm"))]
#[no_mangle]
pub extern "C" fn minijam_accumulate() {}

#[cfg(target_env = "polkavm")]
fn read_current_commitment() -> Result<StateRoot, ()> {
    let mut bytes = [0u8; 34];
    let mut size = 0usize;
    let status = unsafe {
        minijam_storage_read(
            MANAGED_STATE_COMMITMENT_KEY_V1.as_ptr(),
            MANAGED_STATE_COMMITMENT_KEY_V1.len(),
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
    let mut bytes = [0u8; 34];
    let mut size = 0usize;
    let status = unsafe {
        minijam_service_storage_read(
            service_id,
            MANAGED_STATE_COMMITMENT_KEY_V1.as_ptr(),
            MANAGED_STATE_COMMITMENT_KEY_V1.len(),
            bytes.as_mut_ptr(),
            bytes.len(),
            &mut size,
        )
    };
    if status == 0 && size == bytes.len() {
        ManagedStateCommitmentV1::decode(&bytes)
            .map(|commitment| commitment.root)
            .map_err(|_| ())
    } else {
        Err(())
    }
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
    Ok(((first as u64 & (0x7fu64 >> length)) << (8 * length)) | low)
}

#[cfg(target_env = "polkavm")]
fn decode_accumulate_init_input(input: &[u8]) -> Result<(u64, u64, u64), ()> {
    let mut offset = 0usize;
    let tick = read_fnencode(input, &mut offset)?;
    let service_id = read_fnencode(input, &mut offset)?;
    let items_count = read_fnencode(input, &mut offset)?;
    if offset == input.len() {
        Ok((tick, service_id, items_count))
    } else {
        Err(())
    }
}
