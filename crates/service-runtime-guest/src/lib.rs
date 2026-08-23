#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use service_runtime_core::StateDiffV1;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, RuntimeRefineInputV1,
    RuntimeRefineOutputV1, RuntimeRefineOutputV2, ServiceApplication, StateAccessError,
};
use service_runtime_state::ProofState;

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
