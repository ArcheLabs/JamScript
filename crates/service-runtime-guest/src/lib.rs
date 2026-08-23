#![no_std]
#![allow(unexpected_cfgs)]

extern crate alloc;

use alloc::vec::Vec;
use service_runtime_core::StateDiffV1;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, RuntimeRefineInputV1,
    RuntimeRefineOutputV1, RuntimeRefineOutputV2, ServiceApplication, StateAccessError,
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
    }

    #[cfg(not(feature = "diagnostic"))]
    pub fn diagnostic_stage(_message: &'static [u8]) {}

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
}

#[cfg(not(target_env = "polkavm"))]
pub mod guest_support {
    pub struct DiagnosticObserver;
    pub fn diagnostic_stage(_message: &'static [u8]) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestError {
    InvalidInput,
    State,
    Application,
}

pub trait RefineObserver {
    fn stage(&mut self, stage: u8);
}

struct NoopObserver;

impl RefineObserver for NoopObserver {
    fn stage(&mut self, _stage: u8) {}
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
);

pub fn refine<A>(
    application: &A,
    input: &RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV1, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    let (parent_root, new_root, receipts, _, _) = refine_internal(application, input)?;
    Ok(RuntimeRefineOutputV1 {
        version: 1,
        parent_root,
        new_root,
        receipts,
        recovery_commitment: None,
    })
}

pub fn refine_v2<A>(
    application: &A,
    input: &RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV2, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    let (parent_root, new_root, receipts, diff, transition_valid_until) =
        refine_internal(application, input)?;
    RuntimeRefineOutputV2::from_diff_with_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
    )
    .map_err(|_| GuestError::State)
}

pub fn refine_v2_owned<A>(
    application: &A,
    input: RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV2, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    let (parent_root, new_root, receipts, diff, transition_valid_until) =
        refine_internal_owned(application, input)?;
    RuntimeRefineOutputV2::from_diff_with_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
    )
    .map_err(|_| GuestError::State)
}

pub fn refine_v2_owned_with_observer<A, O>(
    application: &A,
    input: RuntimeRefineInputV1,
    observer: &mut O,
) -> Result<RuntimeRefineOutputV2, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
    O: RefineObserver,
{
    let (parent_root, new_root, receipts, diff, transition_valid_until) =
        refine_internal_owned_with_observer(application, input, observer)?;
    RuntimeRefineOutputV2::from_diff_with_validity(
        parent_root,
        new_root,
        receipts,
        diff,
        transition_valid_until,
    )
    .map_err(|_| GuestError::State)
}

fn refine_internal<A>(
    application: &A,
    input: &RuntimeRefineInputV1,
) -> Result<RefineTransition, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    if input.version != 1 || input.managed_state.version != 1 {
        return Err(GuestError::InvalidInput);
    }
    let mut state = ProofState::from_witness(
        input.managed_state.parent_root,
        &input.managed_state.storage_proof,
    )
    .map_err(|_| GuestError::State)?;
    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(input.actions.len());
    let mut transition_valid_until = None;
    for action in &input.actions {
        let action_hash = blake2_256(action);
        state.begin_transaction();
        let (result, action_valid_until) = {
            let mut context = ExecutionContext::new(&mut state, None);
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
            Err(StateAccessError::ApplicationFailed(error_code)) => {
                state.commit_transaction().map_err(|_| GuestError::State)?;
                merge_validity(&mut transition_valid_until, action_valid_until);
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
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
                    error_code: Some(1),
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
    ))
}

fn refine_internal_owned<A>(
    application: &A,
    input: RuntimeRefineInputV1,
) -> Result<RefineTransition, GuestError>
where
    A: ServiceApplication,
    A::Error: Into<StateAccessError>,
{
    let mut observer = NoopObserver;
    refine_internal_owned_with_observer(application, input, &mut observer)
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
    let mut state = ProofState::from_witness_owned_with_observer(
        input.managed_state.parent_root,
        input.managed_state.storage_proof,
        |stage| observer.stage(stage),
    )
    .map_err(|_| GuestError::State)?;
    let parent_root = state.parent_root();
    let mut receipts = Vec::with_capacity(input.actions.len());
    let mut transition_valid_until = None;
    for action in &input.actions {
        let action_hash = blake2_256(action);
        state.begin_transaction();
        observer.stage(STAGE_APPLICATION);
        let (result, action_valid_until) = {
            let mut context = ExecutionContext::new(&mut state, None);
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
            Err(StateAccessError::ApplicationFailed(error_code)) => {
                state.commit_transaction().map_err(|_| GuestError::State)?;
                merge_validity(&mut transition_valid_until, action_valid_until);
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
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
    use service_runtime_core::{ManagedStateWitnessV1, StateChangeV1, StateDiffV1};
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

    fn witness_input(base: &FullState, actions: Vec<Vec<u8>>) -> RuntimeRefineInputV1 {
        RuntimeRefineInputV1 {
            version: 1,
            managed_state: ManagedStateWitnessV1 {
                version: 1,
                parent_root: base.root(),
                storage_proof: base
                    .proof_for(&[b"a", b"b"])
                    .unwrap()
                    .into_nodes()
                    .into_iter()
                    .collect(),
            },
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
                storage_proof: proof.into_nodes().into_iter().collect(),
            },
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
                storage_proof: proof.into_nodes().into_iter().collect(),
            },
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
        let output_v2 = refine_v2(
            &ValidityApplication,
            &witness_input(&base, vec![vec![0], vec![1], vec![2]]),
        )
        .unwrap();
        assert_eq!(output_v2.transition_valid_until, Some(30));
    }

    #[test]
    fn validity_from_rolled_back_action_is_not_committed() {
        let base = FullState::empty();
        let output = refine_v2(
            &ValidityApplication,
            &witness_input(&base, vec![vec![0], vec![3]]),
        )
        .unwrap();
        assert_eq!(output.transition_valid_until, Some(100));
    }
}
