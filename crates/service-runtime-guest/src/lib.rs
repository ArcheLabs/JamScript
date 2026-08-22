#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, RuntimeRefineInputV1,
    RuntimeRefineOutputV1, ServiceApplication, StateAccessError,
};
use service_runtime_state::ProofState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestError {
    InvalidInput,
    State,
    Application,
}

pub fn refine<A>(
    application: &A,
    input: &RuntimeRefineInputV1,
) -> Result<RuntimeRefineOutputV1, GuestError>
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
    for action in &input.actions {
        let action_hash = blake2_256(action);
        state.begin_transaction();
        let result = {
            let mut context = ExecutionContext::new(&mut state, None);
            application
                .execute(&mut context, action)
                .map_err(Into::into)
        };
        match result {
            Ok(()) => {
                state.commit_transaction().map_err(|_| GuestError::State)?;
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
    let (new_root, _) = state.finish().map_err(|_| GuestError::State)?;
    Ok(RuntimeRefineOutputV1 {
        version: 1,
        parent_root,
        new_root,
        receipts,
        recovery_commitment: None,
    })
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
}
