use service_runtime_core::{
    ManagedStateCommitmentV1, StateDiffV1, StateQueryResponseV1, StateRoot, EMPTY_STATE_ROOT_V1,
};
use service_runtime_state::{FullState, StateError, StateTransaction};
use sp_trie::StorageProof;
use std::collections::BTreeMap;

pub struct HostStateSession {
    pub service_id: u32,
    pub parent_root: StateRoot,
    transaction: StateTransaction,
    proof: Option<StorageProof>,
}

pub struct PreparedTransition {
    pub service_id: u32,
    pub parent_root: StateRoot,
    pub new_root: StateRoot,
    pub diff: StateDiffV1,
    pub witness: StorageProof,
}

impl HostStateSession {
    pub fn open(
        service_id: u32,
        parent_root: StateRoot,
        state: FullState,
    ) -> Result<Self, StateError> {
        if state.root() != parent_root {
            return Err(StateError::InvalidRoot);
        }
        Ok(Self {
            service_id,
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
            service_id: self.service_id,
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

    fn root_available(&self, service_id: u32, root: StateRoot) -> Result<bool, Self::Error>;
    fn open(&self, service_id: u32, root: StateRoot) -> Result<FullState, Self::Error>;
    fn get(
        &self,
        service_id: u32,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, Self::Error>;
}

#[derive(Default)]
pub struct MemoryStateProvider {
    states: BTreeMap<u32, BTreeMap<StateRoot, FullState>>,
}

impl MemoryStateProvider {
    pub fn insert(&mut self, service_id: u32, state: FullState) -> StateRoot {
        let root = state.root();
        self.states
            .entry(service_id)
            .or_default()
            .insert(root, state);
        root
    }
}

impl ServiceStateProvider for MemoryStateProvider {
    type Error = StateError;

    fn root_available(&self, service_id: u32, root: StateRoot) -> Result<bool, Self::Error> {
        Ok(root == EMPTY_STATE_ROOT_V1
            || self
                .states
                .get(&service_id)
                .is_some_and(|states| states.contains_key(&root)))
    }

    fn open(&self, service_id: u32, root: StateRoot) -> Result<FullState, Self::Error> {
        if root == EMPTY_STATE_ROOT_V1 {
            return Ok(FullState::empty());
        }
        self.states
            .get(&service_id)
            .and_then(|states| states.get(&root))
            .cloned()
            .ok_or(StateError::InvalidRoot)
    }

    fn get(
        &self,
        service_id: u32,
        root: StateRoot,
        key: &[u8],
    ) -> Result<StateQueryResponseV1, Self::Error> {
        let state = self.open(service_id, root)?;
        let value = state.get(key)?;
        let proof = state.proof_for(&[key])?;
        Ok(StateQueryResponseV1 {
            service_id,
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
    use service_runtime_state::ManagedState;

    #[test]
    fn every_service_starts_at_the_canonical_empty_root() {
        let provider = MemoryStateProvider::default();
        assert!(provider.root_available(42, EMPTY_STATE_ROOT_V1).unwrap());
        let response = provider.get(42, EMPTY_STATE_ROOT_V1, b"missing").unwrap();
        assert_eq!(response.service_id, 42);
        assert_eq!(response.state_root, EMPTY_STATE_ROOT_V1);
        assert_eq!(response.value, None);
        assert!(!response.proof.is_empty());
    }

    #[test]
    fn host_session_records_a_multiproof_from_touched_keys() {
        let state = FullState::from_pairs([
            (b"alice".as_slice(), b"100".as_slice()),
            (b"bob".as_slice(), b"50".as_slice()),
        ])
        .unwrap();
        let root = state.root();
        let mut session = HostStateSession::open(7, root, state).unwrap();
        assert_eq!(
            ManagedState::get(session.transaction(), b"alice").unwrap(),
            Some(b"100".to_vec())
        );
        ManagedState::set(session.transaction(), b"alice", b"90").unwrap();
        let prepared = session.finish().unwrap();
        assert_eq!(prepared.parent_root, root);
        assert!(!prepared.witness.into_nodes().is_empty());
    }
}
