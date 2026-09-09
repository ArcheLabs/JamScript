#![no_std]
#![allow(unexpected_cfgs)]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use service_runtime_core::StateDiffV1;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, ExternalStateAccess,
    ExternalStateDependencyV1, ManagedStateAccess, RuntimeRefineInputV1, RuntimeRefineOutputV1,
    ServiceApplication, StateAccessError,
};
use service_runtime_state::ProofState;

#[cfg(target_env = "polkavm")]
pub mod guest_support {
    use super::{
        RefineObserver, STAGE_APPLICATION, STAGE_APPLICATION_COMMIT, STAGE_APPLICATION_COMMITTED,
        STAGE_APPLICATION_DONE, STAGE_FINISH, STAGE_FINISH_DONE, STAGE_FIRST_TRIE_GET,
        STAGE_PROOF_READY, STAGE_PROOF_STATE, STAGE_STATE_ERROR,
    };
    use core::alloc::{GlobalAlloc, Layout};

    polkavm_derive::min_stack_size!(2 * 1024 * 1024);

    const HEAP_SIZE: usize = if cfg!(feature = "diagnostic") {
        16 * 1024 * 1024
    } else {
        64 * 1024
    };
    static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
    static mut HEAP_OFFSET: usize = 0;

    const C_ALLOCATION_MAGIC: u32 = 0x4a53_4354;

    #[repr(C)]
    struct CAllocationHeader {
        magic: u32,
        freed: u32,
        size: usize,
    }

    #[cfg(feature = "diagnostic")]
    static mut ALLOCATION_COUNT: usize = 0;
    #[cfg(feature = "diagnostic")]
    static mut REQUESTED_BYTES: usize = 0;
    #[cfg(feature = "diagnostic")]
    static mut HIGH_WATER_MARK: usize = 0;

    struct RuntimeAllocator;

    unsafe impl GlobalAlloc for RuntimeAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let base = HEAP.as_mut_ptr() as usize;
            let offset = (HEAP_OFFSET + layout.align() - 1) & !(layout.align() - 1);
            let end = offset.saturating_add(layout.size());
            if end > HEAP_SIZE {
                #[cfg(feature = "diagnostic")]
                diagnostic_trap(0xE001);
                #[cfg(not(feature = "diagnostic"))]
                return core::ptr::null_mut();
            }
            #[cfg(feature = "diagnostic")]
            {
                ALLOCATION_COUNT = ALLOCATION_COUNT.saturating_add(1);
                REQUESTED_BYTES = REQUESTED_BYTES.saturating_add(layout.size());
                HIGH_WATER_MARK = HIGH_WATER_MARK.max(end);
            }
            HEAP_OFFSET = end;
            base.saturating_add(offset) as *mut u8
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    #[global_allocator]
    static ALLOCATOR: RuntimeAllocator = RuntimeAllocator;

    /// Reset all guest allocations at the beginning of a refine invocation.
    ///
    /// The PVM service is single threaded and the host owns the invocation
    /// boundary, so an arena reset is the deterministic lifecycle boundary for
    /// both Rust and ScriptC allocations.
    pub fn reset_runtime() {
        unsafe {
            HEAP_OFFSET = 0;
            #[cfg(feature = "diagnostic")]
            {
                ALLOCATION_COUNT = 0;
                REQUESTED_BYTES = 0;
                HIGH_WATER_MARK = 0;
            }
        }
    }

    #[cfg(feature = "diagnostic")]
    extern "C" {
        fn minijam_host_call(call: u32, args: *const u64) -> u64;
    }

    #[cfg(feature = "diagnostic")]
    #[inline(never)]
    pub fn diagnostic_stage(message: &'static [u8]) {
        let args = [
            1u64,
            0,
            0,
            message.as_ptr() as usize as u64,
            message.len() as u64,
            0,
        ];
        unsafe {
            minijam_host_call(100, args.as_ptr());
        }

        diagnostic_metrics(message);
    }

    #[cfg(feature = "diagnostic")]
    #[inline(never)]
    fn diagnostic_metrics(stage: &[u8]) {
        let mut args = [0u64; 6];
        let gas_remaining = unsafe { minijam_host_call(0, args.as_ptr()) };
        let mut message = [0u8; 192];
        let mut offset = 0usize;

        append_bytes(&mut message, &mut offset, b"jamscript:metrics stage=");
        append_bytes(&mut message, &mut offset, stage);
        append_bytes(&mut message, &mut offset, b" gas_remaining=");
        append_decimal(&mut message, &mut offset, gas_remaining as usize);
        append_bytes(&mut message, &mut offset, b" allocation_count=");
        append_decimal(&mut message, &mut offset, unsafe { ALLOCATION_COUNT });
        append_bytes(&mut message, &mut offset, b" requested_bytes=");
        append_decimal(&mut message, &mut offset, unsafe { REQUESTED_BYTES });
        append_bytes(&mut message, &mut offset, b" high_water_mark=");
        append_decimal(&mut message, &mut offset, unsafe { HIGH_WATER_MARK });

        args[0] = 1;
        args[3] = message.as_ptr() as usize as u64;
        args[4] = offset as u64;
        unsafe {
            minijam_host_call(100, args.as_ptr());
        }
    }

    #[cfg(feature = "diagnostic")]
    fn append_bytes(buffer: &mut [u8], offset: &mut usize, value: &[u8]) {
        for byte in value {
            if *offset == buffer.len() {
                diagnostic_trap(0xE003);
            }
            buffer[*offset] = *byte;
            *offset += 1;
        }
    }

    #[cfg(feature = "diagnostic")]
    fn append_decimal(buffer: &mut [u8], offset: &mut usize, mut value: usize) {
        let mut digits = [0u8; 20];
        let mut count = 0usize;
        loop {
            digits[count] = b'0' + (value % 10) as u8;
            count += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for digit in digits[..count].iter().rev() {
            append_bytes(buffer, offset, core::slice::from_ref(digit));
        }
    }

    #[cfg(not(feature = "diagnostic"))]
    pub fn diagnostic_stage(_message: &'static [u8]) {}

    #[no_mangle]
    #[inline(never)]
    pub unsafe extern "C" fn jamscript_guest_malloc(size: usize) -> *mut u8 {
        let header_size = core::mem::size_of::<CAllocationHeader>();
        let total = match header_size.checked_add(size.max(1)) {
            Some(value) => value,
            None => return core::ptr::null_mut(),
        };
        let layout =
            match Layout::from_size_align(total, core::mem::align_of::<CAllocationHeader>()) {
                Ok(layout) => layout,
                Err(_) => return core::ptr::null_mut(),
            };
        let base = ALLOCATOR.alloc(layout);
        if base.is_null() {
            return base;
        }
        let header = base.cast::<CAllocationHeader>();
        header.write(CAllocationHeader {
            magic: C_ALLOCATION_MAGIC,
            freed: 0,
            size,
        });
        base.add(header_size)
    }

    #[no_mangle]
    #[inline(never)]
    pub unsafe extern "C" fn jamscript_guest_calloc(count: usize, size: usize) -> *mut u8 {
        let total = match count.checked_mul(size) {
            Some(value) => value,
            None => return core::ptr::null_mut(),
        };
        let pointer = jamscript_guest_malloc(total);
        if !pointer.is_null() {
            memset(pointer, 0, total);
        }
        pointer
    }

    #[no_mangle]
    #[inline(never)]
    pub unsafe extern "C" fn jamscript_guest_realloc(pointer: *mut u8, size: usize) -> *mut u8 {
        if pointer.is_null() {
            return jamscript_guest_malloc(size);
        }
        if size == 0 {
            jamscript_guest_free(pointer);
            return core::ptr::null_mut();
        }
        let header_size = core::mem::size_of::<CAllocationHeader>();
        let header = pointer.sub(header_size).cast::<CAllocationHeader>();
        if (*header).magic != C_ALLOCATION_MAGIC || (*header).freed != 0 {
            return core::ptr::null_mut();
        }
        let replacement = jamscript_guest_malloc(size);
        if replacement.is_null() {
            return replacement;
        }
        memcpy(replacement, pointer, (*header).size.min(size));
        jamscript_guest_free(pointer);
        replacement
    }

    #[no_mangle]
    #[inline(never)]
    pub unsafe extern "C" fn jamscript_guest_free(pointer: *mut u8) {
        if pointer.is_null() {
            return;
        }
        let header = pointer
            .sub(core::mem::size_of::<CAllocationHeader>())
            .cast::<CAllocationHeader>();
        if (*header).magic == C_ALLOCATION_MAGIC {
            (*header).freed = 1;
        }
    }

    #[cfg(feature = "diagnostic")]
    #[inline(never)]
    pub fn diagnostic_trap(code: u32) -> ! {
        let message: &'static [u8] = match code {
            0xE001 => b"jamscript:trap=allocator",
            0xE002 => b"jamscript:trap=panic",
            0xE003 => b"jamscript:trap=observer",
            _ => b"jamscript:trap=unknown",
        };
        diagnostic_stage(message);
        unsafe {
            core::arch::asm!(".4byte 0xc0001073", options(noreturn));
        }
    }

    #[cfg(feature = "diagnostic")]
    #[panic_handler]
    fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
        diagnostic_trap(0xE002)
    }

    #[cfg(not(feature = "diagnostic"))]
    #[panic_handler]
    fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
        loop {}
    }

    pub struct DiagnosticObserver;

    impl RefineObserver for DiagnosticObserver {
        fn stage(&mut self, stage: u8) {
            let message = match stage {
                STAGE_PROOF_STATE => b"jamscript:proof-state" as &'static [u8],
                STAGE_FIRST_TRIE_GET => b"jamscript:first-trie-get",
                STAGE_PROOF_READY => b"jamscript:proof-ready",
                STAGE_APPLICATION => b"jamscript:application",
                STAGE_APPLICATION_DONE => b"jamscript:application-done",
                STAGE_APPLICATION_COMMIT => b"jamscript:application-commit",
                STAGE_APPLICATION_COMMITTED => b"jamscript:application-committed",
                STAGE_STATE_ERROR => b"jamscript:state-error",
                STAGE_FINISH => b"jamscript:finish",
                STAGE_FINISH_DONE => b"jamscript:finish-done",
                _ => {
                    #[cfg(feature = "diagnostic")]
                    diagnostic_trap(0xE003);
                    return;
                }
            };
            diagnostic_stage(message);
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn memcpy(
        destination: *mut u8,
        source: *const u8,
        length: usize,
    ) -> *mut u8 {
        let mut index = 0;
        while index < length {
            destination
                .add(index)
                .write(core::ptr::read_volatile(source.add(index)));
            index += 1;
        }
        destination
    }

    #[no_mangle]
    pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, length: usize) -> *mut u8 {
        let mut index = 0;
        while index < length {
            destination.add(index).write_volatile(value as u8);
            index += 1;
        }
        destination
    }

    #[no_mangle]
    pub unsafe extern "C" fn memmove(
        destination: *mut u8,
        source: *const u8,
        length: usize,
    ) -> *mut u8 {
        if (destination as usize) <= (source as usize) {
            let mut index = 0;
            while index < length {
                destination
                    .add(index)
                    .write_volatile(core::ptr::read_volatile(source.add(index)));
                index += 1;
            }
        } else {
            let mut index = length;
            while index != 0 {
                index -= 1;
                destination
                    .add(index)
                    .write_volatile(core::ptr::read_volatile(source.add(index)));
            }
        }
        destination
    }

    #[no_mangle]
    pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, length: usize) -> i32 {
        let mut index = 0;
        while index < length {
            let a = core::ptr::read_volatile(left.add(index));
            let b = core::ptr::read_volatile(right.add(index));
            if a != b {
                return if a < b { -1 } else { 1 };
            }
            index += 1;
        }
        0
    }

    #[no_mangle]
    pub unsafe extern "C" fn strlen(value: *const u8) -> usize {
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        length
    }

    #[no_mangle]
    pub unsafe extern "C" fn strcmp(left: *const u8, right: *const u8) -> i32 {
        let mut index = 0;
        loop {
            let a = *left.add(index);
            let b = *right.add(index);
            if a != b {
                return if a < b { -1 } else { 1 };
            }
            if a == 0 {
                return 0;
            }
            index += 1;
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn stpcpy(destination: *mut u8, source: *const u8) -> *mut u8 {
        let mut index = 0;
        loop {
            let byte = *source.add(index);
            destination.add(index).write(byte);
            if byte == 0 {
                return destination.add(index);
            }
            index += 1;
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn abort() -> ! {
        #[cfg(feature = "diagnostic")]
        diagnostic_trap(0xE004);
        #[cfg(not(feature = "diagnostic"))]
        core::arch::asm!(".4byte 0xc0001073", options(noreturn));
    }

    #[no_mangle]
    pub unsafe extern "C" fn __assert_fail(
        _expression: *const u8,
        _file: *const u8,
        _line: u32,
        _function: *const u8,
    ) -> ! {
        abort()
    }
}

#[cfg(not(target_env = "polkavm"))]
pub mod guest_support {
    pub struct DiagnosticObserver;
    pub fn diagnostic_stage(_message: &'static [u8]) {}
    pub fn reset_runtime() {}
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuestError {
    InvalidInput,
    State,
    Application,
    NeedState(Vec<u8>),
    NeedExternalState { service_id: u32, key: Vec<u8> },
}

pub trait RefineObserver {
    fn stage(&mut self, stage: u8);
}

pub const STAGE_PROOF_STATE: u8 = 1;
pub const STAGE_FIRST_TRIE_GET: u8 = 2;
pub const STAGE_PROOF_READY: u8 = 3;
pub const STAGE_APPLICATION: u8 = 4;
pub const STAGE_FINISH: u8 = 5;
pub const STAGE_FINISH_DONE: u8 = 6;
pub const STAGE_APPLICATION_DONE: u8 = 7;
pub const STAGE_APPLICATION_COMMIT: u8 = 8;
pub const STAGE_APPLICATION_COMMITTED: u8 = 9;
pub const STAGE_STATE_ERROR: u8 = 10;

type RefineTransition = (
    service_runtime_core::StateRoot,
    service_runtime_core::StateRoot,
    Vec<ActionReceiptV1>,
    StateDiffV1,
    Option<u64>,
    Vec<ExternalStateDependencyV1>,
);

struct ExternalProofStates {
    states: BTreeMap<u32, ProofState>,
}

impl ExternalProofStates {
    fn from_input(
        input: &[service_runtime_core::ExternalStateWitnessV1],
    ) -> Result<Self, GuestError> {
        let mut states = BTreeMap::new();
        for witness in input {
            if witness.managed_state.version != service_runtime_core::ManagedStateWitnessV1::VERSION
                || states.contains_key(&witness.service_id)
            {
                return Err(GuestError::InvalidInput);
            }
            let mut state = ProofState::from_witness(
                witness.managed_state.parent_root,
                &witness.managed_state.storage_proof,
            )
            .map_err(|_| GuestError::State)?;
            for key in &witness.managed_state.access_plan.keys {
                ManagedStateAccess::get(&mut state, key).map_err(|_| GuestError::State)?;
            }
            states.insert(witness.service_id, state);
        }
        Ok(Self { states })
    }
}

impl ExternalStateAccess for ExternalProofStates {
    fn get(&mut self, service_id: u32, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        self.states
            .get_mut(&service_id)
            .ok_or(StateAccessError::Backend)?
            .get(key)
    }
}

fn external_dependencies(
    witnesses: &[service_runtime_core::ExternalStateWitnessV1],
) -> Vec<ExternalStateDependencyV1> {
    witnesses
        .iter()
        .map(|witness| ExternalStateDependencyV1 {
            service_id: witness.service_id,
            state_root: witness.managed_state.parent_root,
        })
        .collect()
}

pub fn refine<A>(
    application: &A,
    input: &RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV1, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    let (parent_root, new_root, receipts, diff, transition_valid_until, dependencies) =
        refine_internal(application, input)?;
    RuntimeRefineOutputV1::from_diff_with_dependencies_and_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        dependencies,
        transition_valid_until,
    )
    .map_err(|_| GuestError::State)
}

pub fn refine_owned<A>(
    application: &A,
    input: RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV1, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    refine(application, &input)
}

fn refine_internal<A>(
    application: &A,
    input: &RuntimeRefineInputV1,
) -> Result<RefineTransition, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    if input.version != RuntimeRefineInputV1::VERSION
        || input.managed_state.version != service_runtime_core::ManagedStateWitnessV1::VERSION
    {
        return Err(GuestError::InvalidInput);
    }
    let mut external = ExternalProofStates::from_input(&input.external_state)?;
    let mut state = ProofState::from_witness(
        input.managed_state.parent_root,
        &input.managed_state.storage_proof,
    )
    .map_err(|_| GuestError::State)?;
    for key in &input.managed_state.access_plan.keys {
        state.get(key).map_err(|_| GuestError::State)?;
    }
    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(input.actions.len());
    let mut transition_valid_until = None;
    for action in &input.actions {
        let action_hash = blake2_256(action);
        state.begin_transaction();
        let (result, action_valid_until) = {
            let mut context = ExecutionContext::with_access_plan_and_external_state(
                &mut state,
                None,
                &input.managed_state.access_plan,
                &mut external,
            );
            let result = application
                .execute(&mut context, action)
                .map_err(Into::into);
            (result, context.transition_valid_until())
        };
        match result {
            Ok(()) => {
                state.commit_transaction().map_err(|_| GuestError::State)?;
                merge_validity(&mut transition_valid_until, action_valid_until);
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Applied,
                    error_code: None,
                });
            }
            Err(StateAccessError::MissingWitness | StateAccessError::InvalidProof) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::State);
            }
            Err(StateAccessError::NeedState(key)) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::NeedState(key));
            }
            Err(StateAccessError::ApplicationFailed(error_code)) => {
                if error_code & 0x8000_0000 != 0 {
                    state
                        .rollback_transaction()
                        .map_err(|_| GuestError::State)?;
                } else {
                    state.commit_transaction().map_err(|_| GuestError::State)?;
                    merge_validity(&mut transition_valid_until, action_valid_until);
                }
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
                    error_code: Some(error_code),
                });
            }
            Err(StateAccessError::Rejected(error_code)) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Rejected,
                    error_code: Some(error_code),
                });
            }
            Err(_) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
                    error_code: Some(0x8000_0001),
                });
            }
        }
    }
    let (new_root, diff) = state.finish().map_err(|_| GuestError::State)?;
    Ok((
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
        external_dependencies(&input.external_state),
    ))
}

/// Execute only the application's read/write discovery logic against a
/// bounded planning state. Known keys are represented as absent values; the
/// first unknown key is returned as a structured NeedState result instead of
/// being confused with an invalid proof.
pub fn plan_owned<A>(application: &A, input: RuntimeRefineInputV1) -> Result<(), GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    plan_owned_internal(application, input, None)
}

pub fn plan_owned_with_external<A>(
    application: &A,
    input: RuntimeRefineInputV1,
) -> Result<(), GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    let mut external = PlanningExternalProofStates::new(&input.external_state)?;
    plan_owned_internal(application, input, Some(&mut external))
}

fn plan_owned_internal<A>(
    application: &A,
    input: RuntimeRefineInputV1,
    mut external: Option<&mut PlanningExternalProofStates>,
) -> Result<(), GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    if input.version != RuntimeRefineInputV1::VERSION
        || input.managed_state.version != service_runtime_core::ManagedStateWitnessV1::VERSION
    {
        return Err(GuestError::InvalidInput);
    }
    let mut state = PlanningProofState::new(
        input.managed_state.parent_root,
        input.managed_state.storage_proof,
        &input.managed_state.access_plan.keys,
    )?;
    for action in &input.actions {
        let result = match external.as_deref_mut() {
            Some(external) => {
                let mut context = ExecutionContext::with_access_plan_and_external_state(
                    &mut state,
                    None,
                    &input.managed_state.access_plan,
                    external,
                );
                application
                    .execute(&mut context, action)
                    .map_err(Into::into)
            }
            None => {
                let mut context = ExecutionContext::with_access_plan(
                    &mut state,
                    None,
                    &input.managed_state.access_plan,
                );
                application
                    .execute(&mut context, action)
                    .map_err(Into::into)
            }
        };
        match result {
            Ok(())
            | Err(StateAccessError::Rejected(_))
            | Err(StateAccessError::ApplicationFailed(_)) => {}
            Err(StateAccessError::NeedState(key)) => return Err(GuestError::NeedState(key)),
            Err(StateAccessError::NeedExternalState { service_id, key }) => {
                return Err(GuestError::NeedExternalState { service_id, key })
            }
            Err(StateAccessError::MissingWitness | StateAccessError::InvalidProof) => {
                return Err(GuestError::State)
            }
            Err(_) => return Err(GuestError::Application),
        }
    }
    Ok(())
}

struct PlanningExternalProofStates {
    states: BTreeMap<u32, PlanningProofState>,
}

impl PlanningExternalProofStates {
    fn new(witnesses: &[service_runtime_core::ExternalStateWitnessV1]) -> Result<Self, GuestError> {
        let mut states = BTreeMap::new();
        for witness in witnesses {
            if states.contains_key(&witness.service_id) {
                return Err(GuestError::InvalidInput);
            }
            let state = PlanningProofState::new(
                witness.managed_state.parent_root,
                witness.managed_state.storage_proof.clone(),
                &witness.managed_state.access_plan.keys,
            )
            .map_err(|error| match error {
                GuestError::NeedState(key) => GuestError::NeedExternalState {
                    service_id: witness.service_id,
                    key,
                },
                other => other,
            })?;
            states.insert(witness.service_id, state);
        }
        Ok(Self { states })
    }
}

impl ExternalStateAccess for PlanningExternalProofStates {
    fn get(&mut self, service_id: u32, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        let state = self.states.get_mut(&service_id).ok_or_else(|| {
            StateAccessError::NeedExternalState {
                service_id,
                key: key.to_vec(),
            }
        })?;
        state.get(key).map_err(|error| match error {
            StateAccessError::NeedState(key) => {
                StateAccessError::NeedExternalState { service_id, key }
            }
            other => other,
        })
    }
}

/// A proof-backed state view for planning. The backend supplies a proof for
/// the keys discovered so far; reads outside that explicit plan become a
/// deterministic NeedState response, while reads inside it retain the actual
/// value so second-order accesses can be discovered correctly.
struct PlanningProofState {
    proof: ProofState,
    known: BTreeSet<Vec<u8>>,
}

impl PlanningProofState {
    fn new(
        parent_root: service_runtime_core::StateRoot,
        storage_proof: Vec<Vec<u8>>,
        keys: &[Vec<u8>],
    ) -> Result<Self, GuestError> {
        let mut proof =
            ProofState::from_witness(parent_root, &storage_proof).map_err(|_| GuestError::State)?;
        let known = keys.iter().cloned().collect::<BTreeSet<_>>();
        for key in &known {
            match ManagedStateAccess::get(&mut proof, key) {
                Ok(_) => {}
                Err(StateAccessError::MissingWitness) => {
                    return Err(GuestError::NeedState(key.clone()))
                }
                Err(_) => return Err(GuestError::State),
            }
        }
        Ok(Self { proof, known })
    }

    fn require_known(&self, key: &[u8]) -> Result<(), StateAccessError> {
        self.known
            .contains(key)
            .then_some(())
            .ok_or_else(|| StateAccessError::NeedState(key.to_vec()))
    }
}

impl ManagedStateAccess for PlanningProofState {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        self.require_known(key)?;
        ManagedStateAccess::get(&mut self.proof, key)
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), StateAccessError> {
        self.require_known(key)?;
        ManagedStateAccess::set(&mut self.proof, key, value)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), StateAccessError> {
        self.require_known(key)?;
        ManagedStateAccess::delete(&mut self.proof, key)
    }

    fn begin_transaction(&mut self) -> Result<(), StateAccessError> {
        ManagedStateAccess::begin_transaction(&mut self.proof)
    }

    fn commit_transaction(&mut self) -> Result<(), StateAccessError> {
        ManagedStateAccess::commit_transaction(&mut self.proof)
    }

    fn rollback_transaction(&mut self) -> Result<(), StateAccessError> {
        ManagedStateAccess::rollback_transaction(&mut self.proof)
    }
}

pub fn refine_owned_with_observer<A, O>(
    application: &A,
    input: RuntimeRefineInputV1,
    observer: &mut O,
) -> Result<RuntimeRefineOutputV1, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
    O: RefineObserver,
{
    let (parent_root, new_root, receipts, diff, transition_valid_until, dependencies) =
        refine_internal_owned_with_observer(application, input, observer)?;
    RuntimeRefineOutputV1::from_diff_with_dependencies_and_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        dependencies,
        transition_valid_until,
    )
    .map_err(|_| GuestError::State)
}

fn refine_internal_owned_with_observer<A, O>(
    application: &A,
    input: RuntimeRefineInputV1,
    observer: &mut O,
) -> Result<RefineTransition, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
    O: RefineObserver,
{
    if input.version != 1 || input.managed_state.version != 1 {
        return Err(GuestError::InvalidInput);
    }
    let mut external = ExternalProofStates::from_input(&input.external_state)?;
    let mut state = ProofState::from_witness_owned_with_observer(
        input.managed_state.parent_root,
        input.managed_state.storage_proof,
        |stage| observer.stage(stage),
    )
    .map_err(|_| GuestError::State)?;
    for key in &input.managed_state.access_plan.keys {
        state.get(key).map_err(|_| GuestError::State)?;
    }
    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(input.actions.len());
    let mut transition_valid_until = None;
    for action in &input.actions {
        let action_hash = blake2_256(action);
        state.begin_transaction();
        observer.stage(STAGE_APPLICATION);
        let (result, action_valid_until) = {
            let mut context = ExecutionContext::with_access_plan_and_external_state(
                &mut state,
                None,
                &input.managed_state.access_plan,
                &mut external,
            );
            let result = application
                .execute(&mut context, action)
                .map_err(Into::into);
            (result, context.transition_valid_until())
        };
        observer.stage(STAGE_APPLICATION_DONE);
        match result {
            Ok(()) => {
                observer.stage(STAGE_APPLICATION_COMMIT);
                if state.commit_transaction().is_err() {
                    observer.stage(STAGE_STATE_ERROR);
                    return Err(GuestError::State);
                }
                observer.stage(STAGE_APPLICATION_COMMITTED);
                merge_validity(&mut transition_valid_until, action_valid_until);
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Applied,
                    error_code: None,
                });
            }
            Err(StateAccessError::MissingWitness | StateAccessError::InvalidProof) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::State);
            }
            Err(StateAccessError::NeedState(key)) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                return Err(GuestError::NeedState(key));
            }
            Err(StateAccessError::ApplicationFailed(error_code)) => {
                if error_code & 0x8000_0000 != 0 {
                    guest_support::diagnostic_stage(b"jamscript:application-failed-native");
                    state
                        .rollback_transaction()
                        .map_err(|_| GuestError::State)?;
                } else {
                    guest_support::diagnostic_stage(b"jamscript:application-failed-low");
                    state.commit_transaction().map_err(|_| GuestError::State)?;
                    merge_validity(&mut transition_valid_until, action_valid_until);
                }
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
                    error_code: Some(error_code),
                });
            }
            Err(StateAccessError::Rejected(error_code)) => {
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Rejected,
                    error_code: Some(error_code),
                });
            }
            Err(_) => {
                guest_support::diagnostic_stage(b"jamscript:application-generic-error");
                state
                    .rollback_transaction()
                    .map_err(|_| GuestError::State)?;
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
                    error_code: Some(1),
                });
            }
        }
    }
    observer.stage(STAGE_FINISH);
    let (new_root, diff) = state.finish().map_err(|_| GuestError::State)?;
    observer.stage(STAGE_FINISH_DONE);
    Ok((
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
        external_dependencies(&input.external_state),
    ))
}

fn merge_validity(current: &mut Option<u64>, action: Option<u64>) {
    if let Some(action) = action {
        *current = Some(current.map_or(action, |current| current.min(action)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use service_runtime_core::{
        ManagedStateWitnessV1, StateAccessPlanV1, StateChangeV1, StateDiffV1, StateRecoveryV1,
    };
    use service_runtime_state::FullState;

    struct FailingApplication;

    impl ServiceApplication for FailingApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            context.state().set(b"a", b"1")?;
            context.state().set(b"b", b"2")?;
            Err(StateAccessError::Backend)
        }
    }

    struct NestedFailingApplication;

    impl ServiceApplication for NestedFailingApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            context.state().set(b"nonce", &1u64.to_le_bytes())?;
            context.begin_transaction()?;
            context.state().set(b"a", b"1")?;
            context.state().set(b"b", b"2")?;
            context.rollback_transaction()?;
            Err(StateAccessError::ApplicationFailed(77))
        }
    }

    struct NativeFailingApplication;

    impl ServiceApplication for NativeFailingApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            context.state().set(b"nonce", &1u64.to_le_bytes())?;
            context.begin_transaction()?;
            context.state().set(b"business", b"discard")?;
            context.rollback_transaction()?;
            Err(StateAccessError::ApplicationFailed(0x8000_0009))
        }
    }

    struct ValidityApplication;

    impl ServiceApplication for ValidityApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            match input.first().copied() {
                Some(0) => {
                    context.constrain_valid_until(100);
                    context.state().set(b"a", b"ok")?;
                    Ok(())
                }
                Some(1) => Err(StateAccessError::Backend),
                Some(2) => {
                    context.constrain_valid_until(30);
                    context.state().set(b"b", b"failed")?;
                    Err(StateAccessError::ApplicationFailed(9))
                }
                Some(3) => {
                    context.constrain_valid_until(1);
                    Err(StateAccessError::Backend)
                }
                _ => Err(StateAccessError::Backend),
            }
        }
    }

    struct ExternalReader;

    impl ServiceApplication for ExternalReader {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            let value = context
                .external_get(7, b"external")?
                .ok_or(StateAccessError::Backend)?;
            context.state().set(b"result", &value)
        }
    }

    struct SecondOrderReader;

    impl ServiceApplication for SecondOrderReader {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            let pointer = context
                .state()
                .get(b"first")?
                .ok_or(StateAccessError::Backend)?;
            let value = context
                .state()
                .get(&pointer)?
                .ok_or(StateAccessError::Backend)?;
            context.state().set(b"result", &value)
        }
    }

    #[test]
    fn planner_uses_proven_values_for_second_order_state_access() {
        let base = FullState::from_pairs([
            (b"first".as_slice(), b"second".as_slice()),
            (b"second".as_slice(), b"value".as_slice()),
        ])
        .unwrap();
        let first_only = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: ManagedStateWitnessV1 {
                version: ManagedStateWitnessV1::VERSION,
                parent_root: base.root(),
                access_plan: StateAccessPlanV1::from_keys([b"first".as_slice()]).unwrap(),
                storage_proof: base
                    .proof_for(&[b"first"])
                    .unwrap()
                    .into_nodes()
                    .into_iter()
                    .collect(),
            },
            external_state: Vec::new(),
            actions: vec![vec![]],
        };
        assert_eq!(
            plan_owned(&SecondOrderReader, first_only),
            Err(GuestError::NeedState(b"second".to_vec()))
        );

        let complete = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: ManagedStateWitnessV1 {
                version: ManagedStateWitnessV1::VERSION,
                parent_root: base.root(),
                access_plan: StateAccessPlanV1::from_keys([
                    b"first".as_slice(),
                    b"second".as_slice(),
                    b"result".as_slice(),
                ])
                .unwrap(),
                storage_proof: base
                    .proof_for(&[b"first", b"second", b"result"])
                    .unwrap()
                    .into_nodes()
                    .into_iter()
                    .collect(),
            },
            external_state: Vec::new(),
            actions: vec![vec![]],
        };
        assert_eq!(plan_owned(&SecondOrderReader, complete), Ok(()));
    }

    #[test]
    fn external_state_is_proof_backed_read_only_and_emits_service_id_dependency() {
        let local = FullState::empty();
        let external =
            FullState::from_pairs([(b"external".as_slice(), b"value".as_slice())]).unwrap();
        let input = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: ManagedStateWitnessV1 {
                version: ManagedStateWitnessV1::VERSION,
                parent_root: local.root(),
                access_plan: StateAccessPlanV1::from_keys([b"result".as_slice()]).unwrap(),
                storage_proof: local
                    .proof_for(&[b"result"])
                    .unwrap()
                    .into_nodes()
                    .into_iter()
                    .collect(),
            },
            external_state: vec![service_runtime_core::ExternalStateWitnessV1 {
                service_id: 7,
                managed_state: ManagedStateWitnessV1 {
                    version: ManagedStateWitnessV1::VERSION,
                    parent_root: external.root(),
                    access_plan: StateAccessPlanV1::from_keys([b"external".as_slice()]).unwrap(),
                    storage_proof: external
                        .proof_for(&[b"external"])
                        .unwrap()
                        .into_nodes()
                        .into_iter()
                        .collect(),
                },
            }],
            actions: vec![vec![]],
        };
        let output = refine(&ExternalReader, &input).unwrap();
        assert_eq!(
            output.external_dependencies,
            vec![service_runtime_core::ExternalStateDependencyV1 {
                service_id: 7,
                state_root: external.root(),
            }]
        );
        let recovery = StateRecoveryV1::decode(&output.recovery_payload).unwrap();
        assert_eq!(recovery.diff.changes[0].value, Some(b"value".to_vec()));
    }

    #[test]
    fn invalid_external_proof_is_rejected_before_application_execution() {
        let local = FullState::empty();
        let external =
            FullState::from_pairs([(b"external".as_slice(), b"value".as_slice())]).unwrap();
        let mut nodes: Vec<Vec<u8>> = external
            .proof_for(&[b"external"])
            .unwrap()
            .into_nodes()
            .into_iter()
            .collect();
        nodes[0][0] ^= 1;
        let input = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: ManagedStateWitnessV1 {
                version: ManagedStateWitnessV1::VERSION,
                parent_root: local.root(),
                access_plan: StateAccessPlanV1::from_keys([b"result".as_slice()]).unwrap(),
                storage_proof: local
                    .proof_for(&[b"result"])
                    .unwrap()
                    .into_nodes()
                    .into_iter()
                    .collect(),
            },
            external_state: vec![service_runtime_core::ExternalStateWitnessV1 {
                service_id: 7,
                managed_state: ManagedStateWitnessV1 {
                    version: ManagedStateWitnessV1::VERSION,
                    parent_root: external.root(),
                    access_plan: StateAccessPlanV1::from_keys([b"external".as_slice()]).unwrap(),
                    storage_proof: nodes,
                },
            }],
            actions: vec![vec![]],
        };
        assert_eq!(refine(&ExternalReader, &input), Err(GuestError::State));
    }

    fn witness_input(base: &FullState, actions: Vec<Vec<u8>>) -> RuntimeRefineInputV1 {
        RuntimeRefineInputV1 {
            version: 1,
            managed_state: ManagedStateWitnessV1 {
                version: 1,
                parent_root: base.root(),
                access_plan: StateAccessPlanV1::from_keys([b"a".as_slice(), b"b"]).unwrap(),
                storage_proof: base
                    .proof_for(&[b"a", b"b"])
                    .unwrap()
                    .into_nodes()
                    .into_iter()
                    .collect(),
            },
            external_state: Vec::new(),
            actions,
        }
    }

    #[test]
    fn failed_action_rolls_back_all_business_writes() {
        let base = FullState::empty();
        let parent_root = base.root();
        let proof = base.proof_for(&[b"a", b"b"]).unwrap();
        let input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: ManagedStateWitnessV1 {
                version: 1,
                parent_root,
                access_plan: StateAccessPlanV1::from_keys([b"a".as_slice(), b"b"]).unwrap(),
                storage_proof: proof.into_nodes().into_iter().collect(),
            },
            external_state: Vec::new(),
            actions: vec![b"fail".to_vec()],
        };

        let output = refine(&FailingApplication, &input).unwrap();
        assert_eq!(output.new_root, parent_root);
        assert_eq!(output.receipts[0].status, ActionStatusV1::Failed);

        let expected = base
            .apply_diff(&StateDiffV1 {
                changes: vec![
                    StateChangeV1 {
                        key: b"a".to_vec(),
                        value: Some(b"1".to_vec()),
                    },
                    StateChangeV1 {
                        key: b"b".to_vec(),
                        value: Some(b"2".to_vec()),
                    },
                ],
            })
            .unwrap();
        assert_ne!(output.new_root, expected.root());
        assert_eq!(base.get(b"a").unwrap(), None);
        assert_eq!(base.get(b"b").unwrap(), None);
    }

    #[test]
    fn failed_business_transaction_keeps_authenticated_nonce() {
        let base = FullState::empty();
        let parent_root = base.root();
        let proof = base.proof_for(&[b"nonce", b"a", b"b"]).unwrap();
        let input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: ManagedStateWitnessV1 {
                version: 1,
                parent_root,
                access_plan: StateAccessPlanV1::from_keys([
                    b"nonce".as_slice(),
                    b"a".as_slice(),
                    b"b".as_slice(),
                ])
                .unwrap(),
                storage_proof: proof.into_nodes().into_iter().collect(),
            },
            external_state: Vec::new(),
            actions: vec![b"fail-business".to_vec()],
        };

        let output = refine(&NestedFailingApplication, &input).unwrap();
        assert_eq!(output.receipts[0].error_code, Some(77));
        assert_ne!(output.new_root, parent_root);

        let expected = base
            .apply_diff(&StateDiffV1 {
                changes: vec![StateChangeV1 {
                    key: b"nonce".to_vec(),
                    value: Some(1u64.to_le_bytes().to_vec()),
                }],
            })
            .unwrap();
        assert_eq!(output.new_root, expected.root());
        assert_eq!(base.get(b"a").unwrap(), None);
        assert_eq!(base.get(b"b").unwrap(), None);
    }

    #[test]
    fn native_failure_rolls_back_authenticated_nonce() {
        let base = FullState::empty();
        let parent_root = base.root();
        let proof = base.proof_for(&[b"nonce", b"business"]).unwrap();
        let input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: ManagedStateWitnessV1 {
                version: 1,
                parent_root,
                access_plan: StateAccessPlanV1::from_keys([
                    b"nonce".as_slice(),
                    b"business".as_slice(),
                ])
                .unwrap(),
                storage_proof: proof.into_nodes().into_iter().collect(),
            },
            external_state: Vec::new(),
            actions: vec![b"native-fail".to_vec()],
        };

        let output = refine(&NativeFailingApplication, &input).unwrap();
        assert_eq!(output.new_root, parent_root);
        assert_eq!(output.receipts[0].error_code, Some(0x8000_0009));
    }

    #[test]
    fn validity_merges_committed_actions_and_ignores_rolled_back_actions() {
        let base = FullState::empty();
        let output = refine(
            &ValidityApplication,
            &witness_input(&base, vec![vec![0], vec![1], vec![2]]),
        )
        .unwrap();

        assert_eq!(output.receipts[0].status, ActionStatusV1::Applied);
        assert_eq!(output.receipts[1].status, ActionStatusV1::Failed);
        assert_eq!(output.receipts[2].error_code, Some(9));
        let output_v1 = refine(
            &ValidityApplication,
            &witness_input(&base, vec![vec![0], vec![1], vec![2]]),
        )
        .unwrap();
        assert_eq!(output_v1.transition_valid_until, Some(30));
    }

    #[test]
    fn validity_from_rolled_back_action_is_not_committed() {
        let base = FullState::empty();
        let output = refine(
            &ValidityApplication,
            &witness_input(&base, vec![vec![0], vec![3]]),
        )
        .unwrap();
        assert_eq!(output.transition_valid_until, Some(100));
    }
}
