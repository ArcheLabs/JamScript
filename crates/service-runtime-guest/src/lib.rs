#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, ManagedStateAccess,
    RuntimeRefineInputV1, RuntimeRefineOutputV1, ServiceApplication, StateRoot,
};

pub trait GuestState: ManagedStateAccess {
    fn parent_root(&self) -> StateRoot;
    fn finish_root(&mut self) -> Result<StateRoot, GuestError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestError {
    InvalidInput,
    State,
    Application,
}

pub fn refine<A, S>(
    application: &A,
    state: &mut S,
    input: &RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV1, GuestError>
where
    A: ServiceApplication,
    S: GuestState,
{
    let parent_root = state.parent_root();
    if input.managed_state.parent_root != parent_root {
        return Err(GuestError::InvalidInput);
    }
    let mut receipts = Vec::with_capacity(input.actions.len());
    for action in &input.actions {
        let action_hash = blake2_256(action);
        let mut context = ExecutionContext::new(state, None);
        let result = application
            .execute(&mut context, action)
            .map_err(|_| GuestError::Application);
        receipts.push(ActionReceiptV1 {
            action_hash,
            status: if result.is_ok() {
                ActionStatusV1::Applied
            } else {
                ActionStatusV1::Failed
            },
            error_code: result.err().map(|_| 1),
        });
    }
    let new_root = state.finish_root()?;
    Ok(RuntimeRefineOutputV1 {
        version: 1,
        parent_root,
        new_root,
        receipts,
        recovery_commitment: None,
    })
}
