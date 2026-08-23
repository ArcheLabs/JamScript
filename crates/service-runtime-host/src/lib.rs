use service_runtime_core::{
    blake2_256, ActionReceiptV1, ActionStatusV1, ExecutionContext, ManagedStateCommitmentV1,
    RuntimeRefineInputV1, RuntimeRefineOutputV2, ServiceApplication, ServiceKeyV1,
    StateAccessError, StateAccessPlanV1, StateDiffV1, StateQueryResponseV1, StateRecoveryV1,
    StateRoot, EMPTY_STATE_ROOT_V1, MANAGED_STATE_COMMITMENT_KEY_V1,
};
use service_runtime_guest::refine_v2;
use service_runtime_state::{FullState, StateError, StateTransaction};
use sp_trie::StorageProof;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderError {
    State(StateError),
    UnavailableRoot,
    InvalidRecovery,
}

impl From<StateError> for ProviderError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

pub struct HostStateSession {
    pub service_key: ServiceKeyV1,
    pub parent_root: StateRoot,
    transaction: StateTransaction,
    proof: Option<StorageProof>,
}

pub struct PreparedTransition {
    pub service_key: ServiceKeyV1,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub diff: StateDiffV1,
    pub witness: StorageProof,
}

impl HostStateSession {
    pub fn open(
        service_key: ServiceKeyV1,
        parent_root: StateRoot,
        state: FullState,
    ) -> Result<Self, StateError> {
        if state.root() != parent_root {
            return Err(StateError::InvalidRoot);
        }
        Ok(Self {
            service_key,
            parent_root,
            transaction: StateTransaction::new(state),
            proof: None,
        })
    }

    pub fn transaction(&mut self) -> &mut StateTransaction {
        &mut self.transaction
    }

    pub fn record_proof(&mut self, proof: StorageProof) {
        self.proof = Some(proof);
    }

    pub fn finish(self) -> Result<PreparedTransition, StateError> {
        let (state, diff, recorded_proof) = self.transaction.finish_with_proof()?;
        Ok(PreparedTransition {
            service_key: self.service_key,
            parent_root: self.parent_root,
            new_root: state.root(),
            diff,
            witness: self.proof.unwrap_or(recorded_proof),
        })
    }
}

pub fn commitment_for(root: StateRoot) -> ManagedStateCommitmentV1 {
    ManagedStateCommitmentV1::new(root)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalizedContextV1 {
    pub block_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub slot: u64,
}

pub trait FinalizedManagedStateSource {
    type Error;

    fn finalized_context(&mut self) -> Result<FinalizedContextV1, Self::Error>;
    fn service_storage_at(
        &mut self,
        context: &FinalizedContextV1,
        service: ServiceKeyV1,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltManagedWorkV1 {
    pub context: FinalizedContextV1,
    pub service: ServiceKeyV1,
    pub parent_root: StateRoot,
    pub refine_input: RuntimeRefineInputV1,
    pub predicted_output: RuntimeRefineOutputV2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkBuilderError<SourceError, ProviderError> {
    Source(SourceError),
    Provider(ProviderError),
    InvalidCommitment,
    State(StateError),
    Application(StateAccessError),
    Verification,
    ProducerVerifierMismatch,
}

pub struct ManagedStateWorkBuilder<'a, Source, Provider> {
    source: &'a mut Source,
    provider: &'a Provider,
}

impl<'a, Source, Provider> ManagedStateWorkBuilder<'a, Source, Provider>
where
    Source: FinalizedManagedStateSource,
    Provider: ServiceStateProvider,
{
    pub fn new(source: &'a mut Source, provider: &'a Provider) -> Self {
        Self { source, provider }
    }

    pub fn build_one<Application>(
        &mut self,
        service: ServiceKeyV1,
        application: &Application,
        action: Vec<u8>,
    ) -> Result<BuiltManagedWorkV1, WorkBuilderError<Source::Error, Provider::Error>>
    where
        Application: ServiceApplication,
        Application::Error: Into<StateAccessError>,
    {
        self.build_actions(service, application, vec![action])
    }

    pub fn build_actions<Application>(
        &mut self,
        service: ServiceKeyV1,
        application: &Application,
        actions: Vec<Vec<u8>>,
    ) -> Result<BuiltManagedWorkV1, WorkBuilderError<Source::Error, Provider::Error>>
    where
        Application: ServiceApplication,
        Application::Error: Into<StateAccessError>,
    {
        let context = self
            .source
            .finalized_context()
            .map_err(WorkBuilderError::Source)?;
        let commitment = self
            .source
            .service_storage_at(&context, service, MANAGED_STATE_COMMITMENT_KEY_V1)
            .map_err(WorkBuilderError::Source)?;
        let parent_root = match commitment {
            Some(bytes) => {
                ManagedStateCommitmentV1::decode(&bytes)
                    .map_err(|_| WorkBuilderError::InvalidCommitment)?
                    .root
            }
            None => EMPTY_STATE_ROOT_V1,
        };
        let state = self
            .provider
            .open(service, parent_root)
            .map_err(WorkBuilderError::Provider)?;
        if state.root() != parent_root {
            return Err(WorkBuilderError::State(StateError::InvalidRoot));
        }

        let (predicted_output, proof) = producer_execute(application, state, &actions)?;
        let refine_input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: service_runtime_core::ManagedStateWitnessV1 {
                version: 1,
                parent_root,
                storage_proof: proof.into_nodes().into_iter().collect(),
            },
            actions,
        };
        let verified_output =
            refine_v2(application, &refine_input).map_err(|_| WorkBuilderError::Verification)?;
        if verified_output != predicted_output {
            return Err(WorkBuilderError::ProducerVerifierMismatch);
        }
        Ok(BuiltManagedWorkV1 {
            context,
            service,
            parent_root,
            refine_input,
            predicted_output,
        })
    }
}

fn producer_execute<Application, SourceError, ProviderError>(
    application: &Application,
    state: FullState,
    actions: &[Vec<u8>],
) -> Result<(RuntimeRefineOutputV2, StorageProof), WorkBuilderError<SourceError, ProviderError>>
where
    Application: ServiceApplication,
    Application::Error: Into<StateAccessError>,
{
    let parent_root = state.root();
    let mut state = StateTransaction::new(state);
    let mut receipts = Vec::with_capacity(actions.len());
    let mut transition_valid_until = None;
    for action in actions {
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
                state
                    .commit_transaction()
                    .map_err(WorkBuilderError::State)?;
                merge_validity(&mut transition_valid_until, action_valid_until);
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Applied,
                    error_code: None,
                });
            }
            Err(StateAccessError::ApplicationFailed(error_code)) => {
                if error_code & 0x8000_0000 != 0 {
                    state
                        .rollback_transaction()
                        .map_err(WorkBuilderError::State)?;
                } else {
                    state
                        .commit_transaction()
                        .map_err(WorkBuilderError::State)?;
                    merge_validity(&mut transition_valid_until, action_valid_until);
                }
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
                    error_code: Some(error_code),
                });
            }
            Err(error @ (StateAccessError::MissingWitness | StateAccessError::InvalidProof)) => {
                state
                    .rollback_transaction()
                    .map_err(WorkBuilderError::State)?;
                return Err(WorkBuilderError::Application(error));
            }
            Err(_) => {
                state
                    .rollback_transaction()
                    .map_err(WorkBuilderError::State)?;
                receipts.push(ActionReceiptV1 {
                    action_hash,
                    status: ActionStatusV1::Failed,
                    error_code: Some(1),
                });
            }
        }
    }
    let (next, diff, proof) = state.finish_with_proof().map_err(WorkBuilderError::State)?;
    let output = RuntimeRefineOutputV2::from_diff_with_validity(
        parent_root,
        next.root(),
        receipts,
        diff,
        transition_valid_until,
    )
    .map_err(|_| WorkBuilderError::Verification)?;
    Ok((output, proof))
}

fn merge_validity(current: &mut Option<u64>, action: Option<u64>) {
    if let Some(action) = action {
        *current = Some(current.map_or(action, |existing| existing.min(action)));
    }
}

pub trait ServiceStateProvider {
    type Error;

    /// Returns the last locally materialized root. This is an execution cursor,
    /// not a canonicality claim; callers must obtain canonical roots from JAM.
    fn materialized_root(&self, service: ServiceKeyV1) -> Result<StateRoot, Self::Error>;
    fn build_witness(
        &self,
        service: ServiceKeyV1,
        parent_root: StateRoot,
        plan: &StateAccessPlanV1,
    ) -> Result<service_runtime_core::ManagedStateWitnessV1, Self::Error>;
    fn apply_recovery(
        &mut self,
        service: ServiceKeyV1,
        output: &RuntimeRefineOutputV2,
    ) -> Result<(), Self::Error>;
    fn open(&self, service: ServiceKeyV1, root: StateRoot) -> Result<FullState, Self::Error>;
    fn get(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, Self::Error>;
}

#[derive(Clone, Default)]
pub struct FullStateProvider {
    states: BTreeMap<ServiceKeyV1, BTreeMap<StateRoot, FullState>>,
    materialized: BTreeMap<ServiceKeyV1, StateRoot>,
}

impl FullStateProvider {
    /// Materializes a snapshot identified only by `(service, state root)`.
    /// Insertion does not make the snapshot canonical.
    pub fn insert(&mut self, service: ServiceKeyV1, state: FullState) -> StateRoot {
        let root = state.root();
        self.states.entry(service).or_default().insert(root, state);
        self.materialized.insert(service, root);
        root
    }

    fn state_at(&self, service: ServiceKeyV1, root: StateRoot) -> Result<FullState, ProviderError> {
        if root == EMPTY_STATE_ROOT_V1 {
            return Ok(FullState::empty());
        }
        self.states
            .get(&service)
            .and_then(|states| states.get(&root))
            .cloned()
            .ok_or(ProviderError::UnavailableRoot)
    }
}

pub type MemoryStateProvider = FullStateProvider;

impl ServiceStateProvider for FullStateProvider {
    type Error = ProviderError;

    fn materialized_root(&self, service: ServiceKeyV1) -> Result<StateRoot, Self::Error> {
        Ok(self
            .materialized
            .get(&service)
            .copied()
            .unwrap_or(EMPTY_STATE_ROOT_V1))
    }

    fn build_witness(
        &self,
        service: ServiceKeyV1,
        parent_root: StateRoot,
        plan: &StateAccessPlanV1,
    ) -> Result<service_runtime_core::ManagedStateWitnessV1, Self::Error> {
        let state = self.state_at(service, parent_root)?;
        let keys = plan.keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let proof = state.proof_for(&keys)?;
        Ok(service_runtime_core::ManagedStateWitnessV1 {
            version: 1,
            parent_root,
            storage_proof: proof.into_nodes().into_iter().collect(),
        })
    }

    fn apply_recovery(
        &mut self,
        service: ServiceKeyV1,
        output: &RuntimeRefineOutputV2,
    ) -> Result<(), Self::Error> {
        let recovery = StateRecoveryV1::decode(&output.recovery_payload)
            .map_err(|_| ProviderError::InvalidRecovery)?;
        if recovery
            .commitment()
            .map_err(|_| ProviderError::InvalidRecovery)?
            != output.recovery_commitment
        {
            return Err(ProviderError::InvalidRecovery);
        }
        let state = self.state_at(service, output.parent_root)?;
        let next = state.apply_diff(&recovery.diff)?;
        if next.root() != output.new_root {
            return Err(ProviderError::InvalidRecovery);
        }
        self.states
            .entry(service)
            .or_default()
            .insert(output.new_root, next);
        self.materialized.insert(service, output.new_root);
        Ok(())
    }

    fn open(&self, service: ServiceKeyV1, root: StateRoot) -> Result<FullState, Self::Error> {
        self.state_at(service, root)
    }

    fn get(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, Self::Error> {
        let state = self.open(service, root)?;
        let value = state.get(key)?;
        let proof = state.proof_for(&[key])?;
        Ok(StateQueryResponseV1 {
            service_key: service,
            state_root: root,
            key: key.to_vec(),
            value,
            proof: proof.into_nodes().into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_crypto::SR25519_CONTEXT;
    use jamscript_protocol::SignedActionV2;
    use schnorrkel::{context::signing_context, ExpansionMode, MiniSecretKey};
    use service_runtime_guest::refine_v2;
    use service_runtime_state::{ManagedState, ProofState};
    use std::cell::Cell;
    use std::collections::VecDeque;

    const SERVICE: ServiceKeyV1 = ServiceKeyV1::new([7; 32]);

    struct Counter;

    impl ServiceApplication for Counter {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            let nonce = context.state().get(b"nonce")?.unwrap_or_default();
            let next_nonce = nonce.first().copied().unwrap_or(0).saturating_add(1);
            context.state().set(b"nonce", &[next_nonce])?;
            context.begin_transaction()?;
            if input == b"fail" {
                context.state().set(b"a", b"1")?;
                context.state().set(b"b", b"2")?;
                context.rollback_transaction()?;
                return Err(StateAccessError::ApplicationFailed(77));
            }
            let counter = context.state().get(b"counter")?.unwrap_or_default();
            let next_counter = counter.first().copied().unwrap_or(0).saturating_add(1);
            context.state().set(b"counter", &[next_counter])?;
            context.commit_transaction()
        }
    }

    #[derive(Default)]
    struct TestFinalizedSource {
        contexts: VecDeque<FinalizedContextV1>,
        commitments: BTreeMap<[u8; 32], Option<Vec<u8>>>,
    }

    impl TestFinalizedSource {
        fn push(&mut self, context: FinalizedContextV1, root: Option<StateRoot>) {
            self.commitments.insert(
                context.block_hash,
                root.map(|root| ManagedStateCommitmentV1::new(root).encode().to_vec()),
            );
            self.contexts.push_back(context);
        }
    }

    impl FinalizedManagedStateSource for TestFinalizedSource {
        type Error = ();

        fn finalized_context(&mut self) -> Result<FinalizedContextV1, Self::Error> {
            self.contexts.pop_front().ok_or(())
        }

        fn service_storage_at(
            &mut self,
            context: &FinalizedContextV1,
            _service: ServiceKeyV1,
            key: &[u8],
        ) -> Result<Option<Vec<u8>>, Self::Error> {
            if key != MANAGED_STATE_COMMITMENT_KEY_V1 {
                return Err(());
            }
            self.commitments.get(&context.block_hash).cloned().ok_or(())
        }
    }

    fn finalized(index: u8) -> FinalizedContextV1 {
        FinalizedContextV1 {
            block_hash: [index; 32],
            state_root: [index.wrapping_add(1); 32],
            slot: u64::from(index),
        }
    }

    struct DynamicApplication;

    impl ServiceApplication for DynamicApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            let mut secondary = b"secondary/".to_vec();
            secondary.extend_from_slice(input);
            let current = context.state().get(input)?.unwrap_or_default();
            let _non_inclusion = context.state().get(&secondary)?;
            context.begin_transaction()?;
            context.state().set(
                input,
                &[current.first().copied().unwrap_or(0).saturating_add(1)],
            )?;
            context.state().set(&secondary, b"seen")?;
            context.commit_transaction()
        }
    }

    #[test]
    fn builder_discovers_dynamic_keys_and_verifies_empty_existing_and_historical_roots() {
        let mut provider = FullStateProvider::default();
        let historical = FullState::from_pairs([(b"alice".as_slice(), [4])]).unwrap();
        let historical_root = provider.insert(SERVICE, historical);
        let materialized = FullState::from_pairs([(b"alice".as_slice(), [99])]).unwrap();
        let materialized_root = provider.insert(SERVICE, materialized);
        assert_ne!(historical_root, materialized_root);

        let mut source = TestFinalizedSource::default();
        source.push(finalized(1), Some(historical_root));
        let built = ManagedStateWorkBuilder::new(&mut source, &provider)
            .build_actions(
                SERVICE,
                &DynamicApplication,
                vec![b"alice".to_vec(), b"bob".to_vec()],
            )
            .unwrap();

        assert_eq!(built.parent_root, historical_root);
        assert_eq!(
            built.refine_input.managed_state.parent_root,
            historical_root
        );
        assert!(!built.refine_input.managed_state.storage_proof.is_empty());
        assert_eq!(built.predicted_output.parent_root, historical_root);
        assert_ne!(built.predicted_output.new_root, historical_root);
        assert_eq!(built.predicted_output.receipts.len(), 2);
        assert_eq!(
            provider.materialized_root(SERVICE).unwrap(),
            materialized_root
        );

        let recovery = StateRecoveryV1::decode(&built.predicted_output.recovery_payload).unwrap();
        let keys = recovery
            .diff
            .changes
            .iter()
            .map(|change| change.key.as_slice())
            .collect::<Vec<_>>();
        assert!(keys.contains(&b"alice".as_slice()));
        assert!(keys.contains(&b"bob".as_slice()));
        assert!(keys.contains(&b"secondary/alice".as_slice()));
        assert!(keys.contains(&b"secondary/bob".as_slice()));

        let mut tampered_input = built.refine_input.clone();
        tampered_input.managed_state.storage_proof[0][0] ^= 1;
        assert!(refine_v2(&DynamicApplication, &tampered_input).is_err());

        let mut empty_source = TestFinalizedSource::default();
        empty_source.push(finalized(2), None);
        let empty = ManagedStateWorkBuilder::new(&mut empty_source, &provider)
            .build_one(SERVICE, &DynamicApplication, b"new".to_vec())
            .unwrap();
        assert_eq!(empty.parent_root, EMPTY_STATE_ROOT_V1);
    }

    #[test]
    fn builder_rejects_unavailable_or_tampered_canonical_roots() {
        let provider = FullStateProvider::default();
        let mut unavailable = TestFinalizedSource::default();
        unavailable.push(finalized(3), Some([9; 32]));
        assert_eq!(
            ManagedStateWorkBuilder::new(&mut unavailable, &provider).build_one(
                SERVICE,
                &DynamicApplication,
                b"key".to_vec()
            ),
            Err(WorkBuilderError::Provider(ProviderError::UnavailableRoot))
        );

        let mut invalid = TestFinalizedSource::default();
        let context = finalized(4);
        invalid.contexts.push_back(context);
        invalid
            .commitments
            .insert(context.block_hash, Some(vec![1, 1]));
        assert_eq!(
            ManagedStateWorkBuilder::new(&mut invalid, &provider).build_one(
                SERVICE,
                &DynamicApplication,
                b"key".to_vec()
            ),
            Err(WorkBuilderError::InvalidCommitment)
        );

        struct MismatchedProvider;
        impl ServiceStateProvider for MismatchedProvider {
            type Error = ProviderError;

            fn materialized_root(&self, _service: ServiceKeyV1) -> Result<StateRoot, Self::Error> {
                Ok(EMPTY_STATE_ROOT_V1)
            }

            fn build_witness(
                &self,
                _service: ServiceKeyV1,
                _parent_root: StateRoot,
                _plan: &StateAccessPlanV1,
            ) -> Result<service_runtime_core::ManagedStateWitnessV1, Self::Error> {
                unreachable!()
            }

            fn apply_recovery(
                &mut self,
                _service: ServiceKeyV1,
                _output: &RuntimeRefineOutputV2,
            ) -> Result<(), Self::Error> {
                unreachable!()
            }

            fn open(
                &self,
                _service: ServiceKeyV1,
                _root: StateRoot,
            ) -> Result<FullState, Self::Error> {
                Ok(FullState::empty())
            }

            fn get(
                &self,
                _service: ServiceKeyV1,
                _root: StateRoot,
                _key: &[u8],
            ) -> Result<StateQueryResponseV1, Self::Error> {
                unreachable!()
            }
        }

        let mut tampered = TestFinalizedSource::default();
        tampered.push(finalized(10), Some([10; 32]));
        assert_eq!(
            ManagedStateWorkBuilder::new(&mut tampered, &MismatchedProvider).build_one(
                SERVICE,
                &DynamicApplication,
                b"key".to_vec(),
            ),
            Err(WorkBuilderError::State(StateError::InvalidRoot))
        );
    }

    struct DivergingApplication(Cell<u8>);

    impl ServiceApplication for DivergingApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            _input: &[u8],
        ) -> Result<(), Self::Error> {
            let next = self.0.get().saturating_add(1);
            self.0.set(next);
            context.state().set(b"value", &[next])
        }
    }

    #[test]
    fn builder_rejects_producer_verifier_divergence() {
        let provider = FullStateProvider::default();
        let mut source = TestFinalizedSource::default();
        source.push(finalized(5), None);
        assert_eq!(
            ManagedStateWorkBuilder::new(&mut source, &provider).build_one(
                SERVICE,
                &DivergingApplication(Cell::new(0)),
                Vec::new(),
            ),
            Err(WorkBuilderError::ProducerVerifierMismatch)
        );
    }

    const NETWORK: [u8; 32] = [11; 32];
    const SELECTOR: [u8; 8] = [12; 8];

    struct SignedCounter;

    impl ServiceApplication for SignedCounter {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            raw_action: &[u8],
        ) -> Result<(), Self::Error> {
            let signed = jamscript_runtime_core::decode_signed_action_v2(raw_action)
                .map_err(|_| StateAccessError::Backend)?;
            let verified =
                jamscript_runtime_core::verify_signed_action_v2(signed, NETWORK, SERVICE, SELECTOR)
                    .map_err(|_| StateAccessError::Backend)?;
            let nonce_key = service_runtime_core::wallet_nonce_key_v1(&verified.sender);
            let nonce = context.state().get(&nonce_key)?.unwrap_or_default();
            let expected = match nonce.as_slice() {
                [] => 0,
                bytes if bytes.len() == 8 => {
                    u64::from_le_bytes(bytes.try_into().map_err(|_| StateAccessError::Backend)?)
                }
                _ => return Err(StateAccessError::Backend),
            };
            if verified.nonce != expected || verified.payload.len() != 8 {
                return Err(StateAccessError::Backend);
            }
            context.constrain_valid_until(verified.valid_until);
            context
                .state()
                .set(&nonce_key, &expected.saturating_add(1).to_le_bytes())?;
            context.begin_transaction()?;
            let key = service_runtime_core::application_key_v1(b"counter/v1", &verified.sender)
                .map_err(|_| StateAccessError::Backend)?;
            let increment = u64::from_le_bytes(
                verified
                    .payload
                    .try_into()
                    .map_err(|_| StateAccessError::Backend)?,
            );
            let current = context.state().get(&key)?.unwrap_or_default();
            let current = match current.as_slice() {
                [] => 0,
                bytes if bytes.len() == 8 => {
                    u64::from_le_bytes(bytes.try_into().map_err(|_| StateAccessError::Backend)?)
                }
                _ => return Err(StateAccessError::Backend),
            };
            context
                .state()
                .set(&key, &current.saturating_add(increment).to_le_bytes())?;
            context.commit_transaction()
        }
    }

    fn signed_counter_action(seed: u8, nonce: u64, increment: u64) -> Vec<u8> {
        let keypair = MiniSecretKey::from_bytes(&[seed; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let mut action = SignedActionV2::unsigned(
            NETWORK,
            SERVICE,
            SELECTOR,
            keypair.public.to_bytes(),
            nonce,
            100,
            increment.to_le_bytes().to_vec(),
        )
        .unwrap();
        action.signature = keypair
            .sign(signing_context(SR25519_CONTEXT).bytes(&action.signing_digest()))
            .to_bytes()
            .to_vec();
        action.encode().unwrap()
    }

    #[test]
    fn builder_runs_real_signed_action_v2_nonce_and_business_state_across_roots() {
        let mut provider = FullStateProvider::default();
        let mut source = TestFinalizedSource::default();
        source.push(finalized(6), None);
        let first = ManagedStateWorkBuilder::new(&mut source, &provider)
            .build_one(SERVICE, &SignedCounter, signed_counter_action(7, 0, 3))
            .unwrap();
        assert_eq!(
            first.predicted_output.receipts[0].status,
            ActionStatusV1::Applied
        );
        provider
            .apply_recovery(SERVICE, &first.predicted_output)
            .unwrap();

        source.push(finalized(7), Some(first.predicted_output.new_root));
        let second = ManagedStateWorkBuilder::new(&mut source, &provider)
            .build_one(SERVICE, &SignedCounter, signed_counter_action(7, 1, 4))
            .unwrap();
        assert_eq!(second.parent_root, first.predicted_output.new_root);
        assert_ne!(second.predicted_output.new_root, second.parent_root);
        assert!(second.refine_input.managed_state.storage_proof.len() >= 2);
    }

    #[test]
    fn refreshed_context_rebuilds_root_and_witness() {
        let mut provider = FullStateProvider::default();
        let first_state = FullState::from_pairs([(b"key".as_slice(), [1])]).unwrap();
        let first_root = provider.insert(SERVICE, first_state);
        let second_state = FullState::from_pairs([(b"key".as_slice(), [2])]).unwrap();
        let second_root = provider.insert(SERVICE, second_state);
        let mut source = TestFinalizedSource::default();
        source.push(finalized(8), Some(first_root));
        source.push(finalized(9), Some(second_root));

        let first = ManagedStateWorkBuilder::new(&mut source, &provider)
            .build_one(SERVICE, &DynamicApplication, b"key".to_vec())
            .unwrap();
        let refreshed = ManagedStateWorkBuilder::new(&mut source, &provider)
            .build_one(SERVICE, &DynamicApplication, b"key".to_vec())
            .unwrap();
        assert_eq!(first.parent_root, first_root);
        assert_eq!(refreshed.parent_root, second_root);
        assert_ne!(
            first.refine_input.managed_state.storage_proof,
            refreshed.refine_input.managed_state.storage_proof
        );
    }

    fn run(provider: &FullStateProvider, actions: Vec<&[u8]>) -> RuntimeRefineOutputV2 {
        let parent_root = provider.materialized_root(SERVICE).unwrap();
        let plan =
            StateAccessPlanV1::from_keys([b"nonce".as_slice(), b"counter", b"a", b"b"]).unwrap();
        let witness = provider.build_witness(SERVICE, parent_root, &plan).unwrap();
        let input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: witness,
            actions: actions.into_iter().map(|action| action.to_vec()).collect(),
        };
        refine_v2(&Counter, &input).unwrap()
    }

    #[test]
    fn every_service_starts_at_the_canonical_empty_root() {
        let provider = MemoryStateProvider::default();
        let service = ServiceKeyV1::new([42; 32]);
        assert_eq!(
            provider.materialized_root(service).unwrap(),
            EMPTY_STATE_ROOT_V1
        );
        let response = provider
            .get(service, EMPTY_STATE_ROOT_V1, b"missing")
            .unwrap();
        assert_eq!(response.service_key, service);
        assert_eq!(response.state_root, EMPTY_STATE_ROOT_V1);
        assert_eq!(response.value, None);
        assert!(!response.proof.is_empty());
    }

    fn verify_query(response: &StateQueryResponseV1) -> Result<Option<Vec<u8>>, StateError> {
        let mut state = ProofState::from_witness(response.state_root, &response.proof)?;
        state.get(&response.key)
    }

    #[test]
    fn provider_queries_explicit_historical_roots_with_layout_v1_proofs() {
        let mut provider = FullStateProvider::default();
        let first = FullState::from_pairs([(b"score".as_slice(), b"10".as_slice())]).unwrap();
        let first_root = provider.insert(SERVICE, first.clone());
        let second = FullState::from_pairs([
            (b"score".as_slice(), b"20".as_slice()),
            (b"other".as_slice(), b"1".as_slice()),
        ])
        .unwrap();
        let second_root = provider.insert(SERVICE, second);

        let historical = provider.get(SERVICE, first_root, b"score").unwrap();
        assert_eq!(historical.value, Some(b"10".to_vec()));
        assert_eq!(verify_query(&historical).unwrap(), historical.value);

        let latest_materialized = provider.get(SERVICE, second_root, b"score").unwrap();
        assert_eq!(latest_materialized.value, Some(b"20".to_vec()));
        assert_eq!(
            verify_query(&latest_materialized).unwrap(),
            latest_materialized.value
        );

        let absent = provider.get(SERVICE, first_root, b"missing").unwrap();
        assert_eq!(absent.value, None);
        assert_eq!(verify_query(&absent).unwrap(), None);

        assert_eq!(
            provider.get(SERVICE, [99; 32], b"score"),
            Err(ProviderError::UnavailableRoot)
        );
        assert_eq!(
            provider.get(ServiceKeyV1::new([8; 32]), first_root, b"score"),
            Err(ProviderError::UnavailableRoot)
        );

        let mut tampered = historical;
        tampered.proof[0][0] ^= 1;
        assert!(verify_query(&tampered).is_err());
    }

    #[test]
    fn host_session_records_a_multiproof_from_touched_keys() {
        let state = FullState::from_pairs([
            (b"alice".as_slice(), b"100".as_slice()),
            (b"bob".as_slice(), b"50".as_slice()),
        ])
        .unwrap();
        let root = state.root();
        let mut session = HostStateSession::open(ServiceKeyV1::new([7; 32]), root, state).unwrap();
        assert_eq!(
            ManagedState::get(session.transaction(), b"alice").unwrap(),
            Some(b"100".to_vec())
        );
        ManagedState::set(session.transaction(), b"alice", b"90").unwrap();
        let prepared = session.finish().unwrap();
        assert_eq!(prepared.parent_root, root);
        assert!(!prepared.witness.into_nodes().is_empty());
    }

    #[test]
    fn provider_advances_full_state_from_refine_recovery_and_survives_restart() {
        let mut provider = FullStateProvider::default();
        provider.insert(SERVICE, FullState::empty());
        let output = run(&provider, vec![b"inc", b"inc", b"fail"]);
        assert_eq!(output.receipts.len(), 3);
        assert_eq!(
            output.receipts[2].status,
            service_runtime_core::ActionStatusV1::Failed
        );
        provider.apply_recovery(SERVICE, &output).unwrap();
        assert_eq!(
            provider.materialized_root(SERVICE).unwrap(),
            output.new_root
        );
        let next = provider.open(SERVICE, output.new_root).unwrap();
        assert_eq!(next.get(b"nonce").unwrap(), Some(vec![3]));
        assert_eq!(next.get(b"counter").unwrap(), Some(vec![2]));
        assert_eq!(next.get(b"a").unwrap(), None);
        assert_eq!(next.get(b"b").unwrap(), None);

        let restarted = provider.clone();
        let second = run(&restarted, vec![b"inc"]);
        assert_ne!(second.new_root, output.new_root);
        let mut restarted = restarted;
        restarted.apply_recovery(SERVICE, &second).unwrap();
        assert_eq!(
            restarted
                .open(SERVICE, second.new_root)
                .unwrap()
                .get(b"counter")
                .unwrap(),
            Some(vec![3])
        );
    }

    #[test]
    fn provider_rejects_invalid_witness_stale_root_and_tampered_recovery() {
        let mut provider = FullStateProvider::default();
        provider.insert(
            SERVICE,
            FullState::from_pairs([(b"seed".as_slice(), b"1".as_slice())]).unwrap(),
        );
        let parent_root = provider.materialized_root(SERVICE).unwrap();
        let plan = StateAccessPlanV1::from_keys([
            b"nonce".as_slice(),
            b"counter".as_slice(),
            b"a".as_slice(),
            b"b".as_slice(),
        ])
        .unwrap();
        let mut witness = provider.build_witness(SERVICE, parent_root, &plan).unwrap();
        witness.storage_proof.clear();
        let invalid_input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: witness,
            actions: vec![b"inc".to_vec()],
        };
        assert!(refine_v2(&Counter, &invalid_input).is_err());

        let output = run(&provider, vec![b"inc"]);
        let mut stale = output.clone();
        stale.parent_root = [8; 32];
        assert_eq!(
            provider.apply_recovery(SERVICE, &stale),
            Err(ProviderError::UnavailableRoot)
        );
        assert_eq!(provider.materialized_root(SERVICE).unwrap(), parent_root);

        let mut tampered = output.clone();
        tampered.recovery_payload[0] ^= 1;
        assert_eq!(
            provider.apply_recovery(SERVICE, &tampered),
            Err(ProviderError::InvalidRecovery)
        );
        assert_eq!(provider.materialized_root(SERVICE).unwrap(), parent_root);

        let mut wrong_commitment = output.clone();
        wrong_commitment.recovery_commitment[0] ^= 1;
        assert_eq!(
            provider.apply_recovery(SERVICE, &wrong_commitment),
            Err(ProviderError::InvalidRecovery)
        );
        let mut wrong_new_root = output.clone();
        wrong_new_root.new_root[0] ^= 1;
        assert_eq!(
            provider.apply_recovery(SERVICE, &wrong_new_root),
            Err(ProviderError::InvalidRecovery)
        );
        assert_eq!(provider.materialized_root(SERVICE).unwrap(), parent_root);
    }
}
