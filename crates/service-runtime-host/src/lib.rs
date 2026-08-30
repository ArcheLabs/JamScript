use service_runtime_core::{
    ExecutionContext, ManagedStateAccess, ManagedStateCommitmentV1, RuntimeRefineInputV1,
    RuntimeRefineOutputV1, ServiceApplication, ServiceKeyV1, StateAccessError, StateAccessPlanV1,
    StateDiffV1, StateQueryResponseV1, StateRecoveryV1, StateRoot, EMPTY_STATE_ROOT_V1,
    MANAGED_STATE_COMMITMENT_KEY_V1,
};
use service_runtime_guest::refine;
use service_runtime_state::{FullState, StateError, StateTransaction};
use sp_trie::StorageProof;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderError {
    State(StateError),
    UnavailableRoot,
    InvalidRecovery,
    MalformedResponse,
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
pub struct BuiltManagedWork {
    pub context: FinalizedContextV1,
    pub service: ServiceKeyV1,
    pub parent_root: StateRoot,
    pub refine_input: RuntimeRefineInputV1,
    pub predicted_output: RuntimeRefineOutputV1,
}

pub trait ServiceStateProvider {
    type Error;

    fn value_at(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Self::Error>;

    fn build_witness(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        plan: &StateAccessPlanV1,
    ) -> Result<service_runtime_core::ManagedStateWitnessV1, Self::Error>;

    fn get(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, Self::Error>;
}

/// Optional capabilities for a provider that keeps materialized snapshots.
///
/// The execution builder only depends on [`ServiceStateProvider`].  This
/// extension is retained for the in-memory/reference backend; remote proof
/// providers do not need to implement it.
pub trait MaterializedServiceStateProvider: ServiceStateProvider {
    fn materialized_root(&self, service: ServiceKeyV1) -> Result<StateRoot, Self::Error>;

    fn apply_recovery(
        &mut self,
        service: ServiceKeyV1,
        output: &RuntimeRefineOutputV1,
    ) -> Result<(), Self::Error>;
}

pub struct AuthenticatedWorkBuilder<'a, Source, Provider> {
    source: &'a mut Source,
    provider: &'a Provider,
}

impl<'a, Source, Provider> AuthenticatedWorkBuilder<'a, Source, Provider>
where
    Source: FinalizedManagedStateSource,
    Provider: ServiceStateProvider,
{
    pub fn new(source: &'a mut Source, provider: &'a Provider) -> Self {
        Self { source, provider }
    }

    pub fn build_actions<Application>(
        &mut self,
        service: ServiceKeyV1,
        application: &Application,
        actions: Vec<Vec<u8>>,
    ) -> Result<BuiltManagedWork, WorkBuilderError<Source::Error, Provider::Error>>
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

        let mut keys = initial_runtime_keys(&actions);
        let mut known = BTreeMap::new();
        for key in &keys {
            let value = self
                .provider
                .value_at(service, parent_root, key)
                .map_err(WorkBuilderError::Provider)?;
            known.insert(key.clone(), value);
        }

        let mut planning_result = None;
        for _round in 0..=MAX_PLANNING_ROUNDS {
            let plan =
                StateAccessPlanV1::from_keys(keys.iter()).map_err(WorkBuilderError::StateWire)?;
            let mut planning_state = PlanningState::new(known.clone());
            let result = planning_execute(application, &mut planning_state, &plan, &actions)
                .map_err(WorkBuilderError::Application)?;
            match result {
                PlanningOutcome::NeedState(key) => {
                    if known.contains_key(&key) {
                        return Err(WorkBuilderError::ProviderInconsistent);
                    }
                    let value = self
                        .provider
                        .value_at(service, parent_root, &key)
                        .map_err(WorkBuilderError::Provider)?;
                    known.insert(key.clone(), value);
                    keys.push(key);
                    keys.sort();
                    keys.dedup();
                }
                PlanningOutcome::Complete => {
                    planning_result = Some(());
                    break;
                }
            }
        }
        if planning_result.is_none() {
            return Err(WorkBuilderError::PlanningLimit);
        }

        let plan =
            StateAccessPlanV1::from_keys(keys.iter()).map_err(WorkBuilderError::StateWire)?;
        let witness = self
            .provider
            .build_witness(service, parent_root, &plan)
            .map_err(WorkBuilderError::Provider)?;
        let refine_input = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: witness,
            actions,
        };
        let predicted_output =
            refine(application, &refine_input).map_err(|_| WorkBuilderError::Verification)?;
        Ok(BuiltManagedWork {
            context,
            service,
            parent_root,
            refine_input,
            predicted_output,
        })
    }
}

const MAX_PLANNING_ROUNDS: usize = 4096;

fn initial_runtime_keys(actions: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut keys = Vec::new();
    for action in actions {
        if let Ok(signed) = jamscript_runtime_core::decode_signed_action_v1(action) {
            if signed.public_key.len() == 32 {
                let mut account = [0; 32];
                account.copy_from_slice(signed.public_key);
                keys.push(service_runtime_core::wallet_nonce_key_v1(&account));
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

enum PlanningOutcome {
    Complete,
    NeedState(Vec<u8>),
}

fn planning_execute<Application>(
    application: &Application,
    state: &mut PlanningState,
    plan: &StateAccessPlanV1,
    actions: &[Vec<u8>],
) -> Result<PlanningOutcome, StateAccessError>
where
    Application: ServiceApplication,
    Application::Error: Into<StateAccessError>,
{
    for action in actions {
        state.begin_transaction()?;
        let result = {
            let mut context = ExecutionContext::with_access_plan(state, None, plan);
            application
                .execute(&mut context, action)
                .map_err(Into::into)
        };
        match result {
            Ok(()) => state.commit_transaction()?,
            Err(StateAccessError::NeedState(key)) => {
                let _ = state.rollback_transaction();
                return Ok(PlanningOutcome::NeedState(key));
            }
            Err(StateAccessError::ApplicationFailed(code)) if code & 0x8000_0000 == 0 => {
                state.commit_transaction()?;
            }
            Err(_) => {
                let _ = state.rollback_transaction();
            }
        }
    }
    Ok(PlanningOutcome::Complete)
}

struct PlanningState {
    base: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    transactions: Vec<BTreeMap<Vec<u8>, Option<Vec<u8>>>>,
}

impl PlanningState {
    fn new(base: BTreeMap<Vec<u8>, Option<Vec<u8>>>) -> Self {
        Self {
            base,
            writes: BTreeMap::new(),
            transactions: Vec::new(),
        }
    }

    fn lookup(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        for transaction in self.transactions.iter().rev() {
            if let Some(value) = transaction.get(key) {
                return Ok(value.clone());
            }
        }
        if let Some(value) = self.writes.get(key) {
            return Ok(value.clone());
        }
        self.base
            .get(key)
            .cloned()
            .ok_or_else(|| StateAccessError::NeedState(key.to_vec()))
    }
}

impl ManagedStateAccess for PlanningState {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        self.lookup(key)
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), StateAccessError> {
        let _ = self.lookup(key)?;
        let target = self
            .transactions
            .last_mut()
            .ok_or(StateAccessError::Backend)?;
        target.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), StateAccessError> {
        let _ = self.lookup(key)?;
        let target = self
            .transactions
            .last_mut()
            .ok_or(StateAccessError::Backend)?;
        target.insert(key.to_vec(), None);
        Ok(())
    }

    fn begin_transaction(&mut self) -> Result<(), StateAccessError> {
        self.transactions.push(BTreeMap::new());
        Ok(())
    }

    fn commit_transaction(&mut self) -> Result<(), StateAccessError> {
        let changes = self.transactions.pop().ok_or(StateAccessError::Backend)?;
        if let Some(parent) = self.transactions.last_mut() {
            parent.extend(changes);
        } else {
            self.writes.extend(changes);
        }
        Ok(())
    }

    fn rollback_transaction(&mut self) -> Result<(), StateAccessError> {
        self.transactions.pop().ok_or(StateAccessError::Backend)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkBuilderError<SourceError, ProviderError> {
    Source(SourceError),
    Provider(ProviderError),
    InvalidCommitment,
    State(StateError),
    Application(StateAccessError),
    StateWire(service_runtime_core::WireError),
    Verification,
    ProviderInconsistent,
    PlanningLimit,
}

#[derive(Clone, Default)]
pub struct FullStateProvider {
    states: BTreeMap<ServiceKeyV1, BTreeMap<StateRoot, FullState>>,
    materialized: BTreeMap<ServiceKeyV1, StateRoot>,
}

impl FullStateProvider {
    pub fn open(&self, service: ServiceKeyV1, root: StateRoot) -> Result<FullState, ProviderError> {
        self.state_at(service, root)
    }

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

    fn value_at(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(self.state_at(service, root)?.get(key)?)
    }

    fn build_witness(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        plan: &StateAccessPlanV1,
    ) -> Result<service_runtime_core::ManagedStateWitnessV1, Self::Error> {
        let state = self.state_at(service, root)?;
        let keys = plan.keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let proof = state.proof_for(&keys)?;
        Ok(service_runtime_core::ManagedStateWitnessV1 {
            version: service_runtime_core::ManagedStateWitnessV1::VERSION,
            parent_root: root,
            access_plan: plan.clone(),
            storage_proof: proof.into_nodes().into_iter().collect(),
        })
    }

    fn get(
        &self,
        service: ServiceKeyV1,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, Self::Error> {
        let state = self.state_at(service, root)?;
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

impl MaterializedServiceStateProvider for FullStateProvider {
    fn materialized_root(&self, service: ServiceKeyV1) -> Result<StateRoot, Self::Error> {
        Ok(self
            .materialized
            .get(&service)
            .copied()
            .unwrap_or(EMPTY_STATE_ROOT_V1))
    }

    fn apply_recovery(
        &mut self,
        service: ServiceKeyV1,
        output: &RuntimeRefineOutputV1,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_crypto::SR25519_CONTEXT;
    use jamscript_protocol::SignedActionV1;
    use schnorrkel::{context::signing_context, ExpansionMode, MiniSecretKey};
    use service_runtime_core::ActionStatusV1;
    use service_runtime_guest::refine;
    use service_runtime_state::{ManagedState, ProofState};
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

    struct PlanningProbe;

    impl ServiceApplication for PlanningProbe {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            let _ = context.state().get(input)?;
            Ok(())
        }
    }

    #[test]
    fn authenticated_builder_replays_until_dynamic_key_is_known() {
        let provider = FullStateProvider::default();
        let mut source = TestFinalizedSource::default();
        source.push(finalized(11), None);
        let built = AuthenticatedWorkBuilder::new(&mut source, &provider)
            .build_actions(SERVICE, &PlanningProbe, vec![b"dynamic-key".to_vec()])
            .unwrap();
        assert_eq!(built.refine_input.version, RuntimeRefineInputV1::VERSION);
        assert_eq!(
            built.refine_input.managed_state.access_plan.keys,
            vec![b"dynamic-key".to_vec()]
        );
        assert_eq!(
            built.predicted_output.receipts[0].status,
            ActionStatusV1::Applied
        );
    }

    struct ChainedDynamicApplication;

    impl ServiceApplication for ChainedDynamicApplication {
        type Error = StateAccessError;

        fn execute(
            &self,
            context: &mut ExecutionContext<'_>,
            input: &[u8],
        ) -> Result<(), Self::Error> {
            let next = context
                .state()
                .get(input)?
                .ok_or(StateAccessError::Backend)?;
            let _ = context.state().get(&next)?;
            context.state().set(&next, b"updated")
        }
    }

    #[test]
    fn authenticated_builder_replays_chained_dynamic_access_and_writes() {
        let mut provider = FullStateProvider::default();
        let state = FullState::from_pairs([
            (b"first".as_slice(), b"second".as_slice()),
            (b"second".as_slice(), b"value".as_slice()),
        ])
        .unwrap();
        let root = provider.insert(SERVICE, state);
        let mut source = TestFinalizedSource::default();
        source.push(finalized(12), Some(root));

        let built = AuthenticatedWorkBuilder::new(&mut source, &provider)
            .build_actions(SERVICE, &ChainedDynamicApplication, vec![b"first".to_vec()])
            .unwrap();
        assert_eq!(
            built.refine_input.managed_state.access_plan.keys,
            vec![b"first".to_vec(), b"second".to_vec()]
        );
        assert_eq!(
            StateRecoveryV1::decode(&built.predicted_output.recovery_payload)
                .unwrap()
                .diff
                .changes,
            vec![service_runtime_core::StateChangeV1 {
                key: b"second".to_vec(),
                value: Some(b"updated".to_vec()),
            }]
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
            let signed = jamscript_runtime_core::decode_signed_action_v1(raw_action)
                .map_err(|_| StateAccessError::Backend)?;
            let verified =
                jamscript_runtime_core::verify_signed_action_v1(signed, NETWORK, SERVICE, SELECTOR)
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
        let mut action = SignedActionV1::unsigned(
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
    fn builder_runs_real_formal_signed_action_nonce_and_business_state_across_roots() {
        let mut provider = FullStateProvider::default();
        let mut source = TestFinalizedSource::default();
        source.push(finalized(6), None);
        let first = AuthenticatedWorkBuilder::new(&mut source, &provider)
            .build_actions(
                SERVICE,
                &SignedCounter,
                vec![signed_counter_action(7, 0, 3)],
            )
            .unwrap();
        assert_eq!(
            first.predicted_output.receipts[0].status,
            ActionStatusV1::Applied
        );
        provider
            .apply_recovery(SERVICE, &first.predicted_output)
            .unwrap();

        source.push(finalized(7), Some(first.predicted_output.new_root));
        let second = AuthenticatedWorkBuilder::new(&mut source, &provider)
            .build_actions(
                SERVICE,
                &SignedCounter,
                vec![signed_counter_action(7, 1, 4)],
            )
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

        let first = AuthenticatedWorkBuilder::new(&mut source, &provider)
            .build_actions(SERVICE, &DynamicApplication, vec![b"key".to_vec()])
            .unwrap();
        let refreshed = AuthenticatedWorkBuilder::new(&mut source, &provider)
            .build_actions(SERVICE, &DynamicApplication, vec![b"key".to_vec()])
            .unwrap();
        assert_eq!(first.parent_root, first_root);
        assert_eq!(refreshed.parent_root, second_root);
        assert_ne!(
            first.refine_input.managed_state.storage_proof,
            refreshed.refine_input.managed_state.storage_proof
        );
    }

    fn run(provider: &FullStateProvider, actions: Vec<&[u8]>) -> RuntimeRefineOutputV1 {
        let parent_root = provider.materialized_root(SERVICE).unwrap();
        let plan =
            StateAccessPlanV1::from_keys([b"nonce".as_slice(), b"counter", b"a", b"b"]).unwrap();
        let witness = provider.build_witness(SERVICE, parent_root, &plan).unwrap();
        let input = RuntimeRefineInputV1 {
            version: 1,
            managed_state: witness,
            actions: actions.into_iter().map(|action| action.to_vec()).collect(),
        };
        refine(&Counter, &input).unwrap()
    }

    #[test]
    fn every_service_starts_at_the_canonical_empty_root() {
        let provider = MemoryStateProvider::default();
        let service = ServiceKeyV1::new([42; 32]);
        assert_eq!(
            provider.materialized_root(service).unwrap(),
            EMPTY_STATE_ROOT_V1
        );
        let response =
            ServiceStateProvider::get(&provider, service, EMPTY_STATE_ROOT_V1, b"missing").unwrap();
        assert_eq!(response.service_key, service);
        assert_eq!(response.state_root, EMPTY_STATE_ROOT_V1);
        assert_eq!(response.value, None);
        assert!(!response.proof.is_empty());
    }

    fn verify_query(response: &StateQueryResponseV1) -> Result<Option<Vec<u8>>, StateError> {
        let mut state = ProofState::from_witness(response.state_root, &response.proof)?;
        ManagedState::get(&mut state, &response.key)
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

        let historical =
            ServiceStateProvider::get(&provider, SERVICE, first_root, b"score").unwrap();
        assert_eq!(historical.value, Some(b"10".to_vec()));
        assert_eq!(verify_query(&historical).unwrap(), historical.value);

        let latest_materialized =
            ServiceStateProvider::get(&provider, SERVICE, second_root, b"score").unwrap();
        assert_eq!(latest_materialized.value, Some(b"20".to_vec()));
        assert_eq!(
            verify_query(&latest_materialized).unwrap(),
            latest_materialized.value
        );

        let absent = ServiceStateProvider::get(&provider, SERVICE, first_root, b"missing").unwrap();
        assert_eq!(absent.value, None);
        assert_eq!(verify_query(&absent).unwrap(), None);

        assert_eq!(
            ServiceStateProvider::get(&provider, SERVICE, [99; 32], b"score"),
            Err(ProviderError::UnavailableRoot)
        );
        assert_eq!(
            ServiceStateProvider::get(&provider, ServiceKeyV1::new([8; 32]), first_root, b"score"),
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
        assert!(refine(&Counter, &invalid_input).is_err());

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
