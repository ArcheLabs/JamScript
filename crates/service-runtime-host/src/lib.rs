use service_runtime_core::{
    ManagedStateCommitmentV1, RuntimeRefineOutputV2, ServiceKeyV1, StateAccessPlanV1, StateDiffV1,
    StateQueryResponseV1, StateRecoveryV1, StateRoot, EMPTY_STATE_ROOT_V1,
};
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
    use service_runtime_core::{
        ExecutionContext, RuntimeRefineInputV1, ServiceApplication, StateAccessError,
    };
    use service_runtime_guest::refine_v2;
    use service_runtime_state::{ManagedState, ProofState};

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
