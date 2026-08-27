#![cfg_attr(not(feature = "std"), no_std)]

//! Stable application boundary for managed service state.
//!
//! This crate deliberately contains no networking or sequencing policy.  A
//! host supplies a commitment and a witness; a service verifies the witness
//! and then uses the existing `ProofState` adapter.

extern crate alloc;
use alloc::vec::Vec;
pub use service_runtime_core::{ManagedStateCommitmentV1, StateRoot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateReadRequest {
    pub keys: Vec<Vec<u8>>,
}
impl StateReadRequest {
    pub fn new(keys: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateWitness {
    pub parent_root: StateRoot,
    pub nodes: Vec<Vec<u8>>,
}
impl StateWitness {
    pub fn new(parent_root: StateRoot, nodes: Vec<Vec<u8>>) -> Self {
        Self { parent_root, nodes }
    }
}

pub trait StateProvider {
    type Error;
    fn read(
        &mut self,
        request: &StateReadRequest,
        witness: &StateWitness,
    ) -> Result<Vec<Option<Vec<u8>>>, Self::Error>;
}

#[cfg(feature = "std")]
pub struct ProofStateProvider {
    state: service_runtime_state::ProofState,
}

#[cfg(feature = "std")]
impl ProofStateProvider {
    pub fn from_witness(
        commitment: ManagedStateCommitmentV1,
        witness: StateWitness,
    ) -> Result<Self, service_runtime_state::StateError> {
        if commitment.root != witness.parent_root {
            return Err(service_runtime_state::StateError::InvalidRoot);
        }
        Ok(Self {
            state: service_runtime_state::ProofState::from_witness_owned(
                witness.parent_root,
                witness.nodes,
            )?,
        })
    }
    pub fn into_state(self) -> service_runtime_state::ProofState {
        self.state
    }
}

#[cfg(feature = "std")]
impl StateProvider for ProofStateProvider {
    type Error = service_runtime_state::StateError;
    fn read(
        &mut self,
        request: &StateReadRequest,
        _witness: &StateWitness,
    ) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        request
            .keys
            .iter()
            .map(|key| service_runtime_state::ManagedState::get(&mut self.state, key))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_runtime_state::FullState;
    #[test]
    fn adapter_verifies_commitment_and_reads_proof_state() {
        let base = FullState::from_pairs([(b"alpha", b"one")]).unwrap();
        let root = base.root();
        let proof = base.proof_for(&[b"alpha", b"missing"]).unwrap();
        let mut provider = ProofStateProvider::from_witness(
            ManagedStateCommitmentV1::new(root),
            StateWitness::new(root, proof.into_iter_nodes().collect()),
        )
        .unwrap();
        let values = provider
            .read(
                &StateReadRequest::new([b"alpha".to_vec(), b"missing".to_vec()]),
                &StateWitness::new(root, Vec::new()),
            )
            .unwrap();
        assert_eq!(values, vec![Some(b"one".to_vec()), None]);
        assert!(ProofStateProvider::from_witness(
            ManagedStateCommitmentV1::new([9; 32]),
            StateWitness::new(root, Vec::new())
        )
        .is_err());
    }
}
