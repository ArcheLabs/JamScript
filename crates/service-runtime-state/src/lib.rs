#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::{collections::BTreeMap, vec::Vec};
use service_runtime_core::{
    ManagedStateAccess, StateAccessError, StateChangeV1, StateDiffV1, StateRoot,
};

#[cfg(feature = "std")]
use alloc::collections::BTreeSet;
use sp_core::{Blake2Hasher, H256};
#[cfg(feature = "std")]
use sp_trie::recorder_ext::RecorderExt;
#[cfg(feature = "std")]
use sp_trie::Recorder;
use sp_trie::TrieConfiguration;
use sp_trie::{LayoutV1, MemoryDB, StorageProof, Trie, TrieDBBuilder, TrieDBMutBuilder, TrieMut};

pub trait ManagedState {
    type Error;

    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error>;
    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error>;
    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    InvalidRoot,
    MissingWitness,
    InvalidProof,
    Backend,
    Unsupported,
}

pub type TrieLayout = LayoutV1<Blake2Hasher>;

pub fn empty_state_root() -> StateRoot {
    root_bytes(TrieLayout::trie_root(core::iter::empty::<(&[u8], &[u8])>()))
}

fn root_bytes(root: H256) -> StateRoot {
    root.as_bytes().try_into().expect("H256 is 32 bytes")
}

fn root_hash(root: StateRoot) -> H256 {
    H256::from(root)
}

#[cfg(feature = "std")]
#[derive(Clone)]
pub struct FullState {
    db: MemoryDB<Blake2Hasher>,
    root: H256,
}

#[cfg(feature = "std")]
impl FullState {
    pub fn empty() -> Self {
        Self {
            db: MemoryDB::default(),
            root: root_hash(empty_state_root()),
        }
    }

    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, StateError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let mut state = Self::empty();
        {
            let mut trie =
                TrieDBMutBuilder::<TrieLayout>::new(&mut state.db, &mut state.root).build();
            for (key, value) in pairs {
                trie.insert(key.as_ref(), value.as_ref())
                    .map_err(|_| StateError::Backend)?;
            }
        }
        Ok(state)
    }

    pub fn root(&self) -> StateRoot {
        root_bytes(self.root)
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StateError> {
        let trie = TrieDBBuilder::<TrieLayout>::new(&self.db, &self.root).build();
        trie.get(key)
            .map(|value| value.map(|value| value.to_vec()))
            .map_err(|_| StateError::Backend)
    }

    pub fn proof_for(&self, keys: &[&[u8]]) -> Result<StorageProof, StateError> {
        let mut recorder = Recorder::<TrieLayout>::new();
        {
            let trie = TrieDBBuilder::<TrieLayout>::new(&self.db, &self.root)
                .with_recorder(&mut recorder)
                .build();
            for key in keys {
                trie.get(key).map_err(|_| StateError::Backend)?;
            }
        }
        Ok(StorageProof::new(recorder.into_raw_storage_proof()))
    }

    pub fn apply_diff(&self, diff: &StateDiffV1) -> Result<Self, StateError> {
        let mut next = self.clone();
        {
            let mut trie =
                TrieDBMutBuilder::<TrieLayout>::new(&mut next.db, &mut next.root).build();
            for change in &diff.changes {
                match &change.value {
                    Some(value) => {
                        let _ = trie
                            .insert(&change.key, value)
                            .map_err(|_| StateError::Backend)?;
                    }
                    None => {
                        let _ = trie.remove(&change.key).map_err(|_| StateError::Backend)?;
                    }
                }
            }
        }
        Ok(next)
    }
}

#[cfg(feature = "std")]
pub struct StateTransaction {
    base: FullState,
    writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    touched: BTreeSet<Vec<u8>>,
}

#[cfg(feature = "std")]
impl StateTransaction {
    pub fn new(base: FullState) -> Self {
        Self {
            base,
            writes: BTreeMap::new(),
            touched: BTreeSet::new(),
        }
    }

    pub fn finish(self) -> Result<(FullState, StateDiffV1), StateError> {
        let (state, diff, _) = self.finish_with_proof()?;
        Ok((state, diff))
    }

    pub fn finish_with_proof(self) -> Result<(FullState, StateDiffV1, StorageProof), StateError> {
        let proof = self
            .base
            .proof_for(&self.touched.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
        let changes = self
            .writes
            .into_iter()
            .map(|(key, value)| StateChangeV1 { key, value })
            .collect();
        let diff = StateDiffV1 { changes };
        let next = self.base.apply_diff(&diff)?;
        Ok((next, diff, proof))
    }

    pub fn discard(self) {}
}

#[cfg(feature = "std")]
impl ManagedState for StateTransaction {
    type Error = StateError;

    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        match self.writes.get(key) {
            Some(value) => Ok(value.clone()),
            None => {
                self.touched.insert(key.to_vec());
                self.base.get(key)
            }
        }
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.touched.insert(key.to_vec());
        self.writes.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error> {
        self.touched.insert(key.to_vec());
        self.writes.insert(key.to_vec(), None);
        Ok(())
    }
}

#[cfg(feature = "std")]
impl ManagedStateAccess for StateTransaction {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        ManagedState::get(self, key).map_err(|_| StateAccessError::Backend)
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), StateAccessError> {
        ManagedState::set(self, key, value).map_err(|_| StateAccessError::Backend)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), StateAccessError> {
        ManagedState::delete(self, key).map_err(|_| StateAccessError::Backend)
    }
}

pub struct ProofState {
    db: MemoryDB<Blake2Hasher>,
    root: H256,
    writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    transactions: Vec<BTreeMap<Vec<u8>, Option<Vec<u8>>>>,
}

impl ProofState {
    pub fn from_witness(root: StateRoot, proof: &[Vec<u8>]) -> Result<Self, StateError> {
        Self::from_proof(root, StorageProof::new(proof.to_vec()))
    }

    pub fn from_proof(root: StateRoot, proof: StorageProof) -> Result<Self, StateError> {
        let db = proof.into_memory_db::<Blake2Hasher>();
        let root = root_hash(root);
        let trie = TrieDBBuilder::<TrieLayout>::new(&db, &root).build();
        let _ = trie.get(&[]).map_err(|_| StateError::MissingWitness)?;
        Ok(Self {
            db,
            root,
            writes: BTreeMap::new(),
            transactions: Vec::new(),
        })
    }

    pub fn parent_root(&self) -> StateRoot {
        root_bytes(self.root)
    }

    pub fn begin_transaction(&mut self) {
        self.transactions.push(BTreeMap::new());
    }

    pub fn commit_transaction(&mut self) -> Result<(), StateError> {
        let changes = self.transactions.pop().ok_or(StateError::Backend)?;
        if let Some(parent) = self.transactions.last_mut() {
            parent.extend(changes);
        } else {
            self.writes.extend(changes);
        }
        Ok(())
    }

    pub fn rollback_transaction(&mut self) -> Result<(), StateError> {
        self.transactions.pop().ok_or(StateError::Backend)?;
        Ok(())
    }

    pub fn finish(self) -> Result<(StateRoot, StateDiffV1), StateError> {
        if !self.transactions.is_empty() {
            return Err(StateError::Backend);
        }
        let diff = StateDiffV1 {
            changes: self
                .writes
                .into_iter()
                .map(|(key, value)| StateChangeV1 { key, value })
                .collect(),
        };
        let mut db = self.db;
        let mut root = self.root;
        {
            let mut trie = TrieDBMutBuilder::<TrieLayout>::new(&mut db, &mut root).build();
            for change in &diff.changes {
                match &change.value {
                    Some(value) => {
                        trie.insert(&change.key, value)
                            .map_err(|_| StateError::Backend)?;
                    }
                    None => {
                        let _ = trie.remove(&change.key).map_err(|_| StateError::Backend)?;
                    }
                }
            }
        }
        Ok((root_bytes(root), diff))
    }

    fn overlay_value(&self, key: &[u8]) -> Option<Option<Vec<u8>>> {
        for transaction in self.transactions.iter().rev() {
            if let Some(value) = transaction.get(key) {
                return Some(value.clone());
            }
        }
        self.writes.get(key).cloned()
    }

    fn ensure_witness(&self, key: &[u8]) -> Result<(), StateError> {
        let trie = TrieDBBuilder::<TrieLayout>::new(&self.db, &self.root).build();
        trie.get(key)
            .map(|_| ())
            .map_err(|_| StateError::MissingWitness)
    }
}

impl ManagedState for ProofState {
    type Error = StateError;

    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        if let Some(value) = self.overlay_value(key) {
            return Ok(value);
        }
        let trie = TrieDBBuilder::<TrieLayout>::new(&self.db, &self.root).build();
        trie.get(key)
            .map(|value| value.map(|value| value.to_vec()))
            .map_err(|_| StateError::MissingWitness)
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        if self.overlay_value(key).is_none() {
            self.ensure_witness(key)?;
        }
        let target = self.transactions.last_mut().unwrap_or(&mut self.writes);
        target.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), Self::Error> {
        if self.overlay_value(key).is_none() {
            self.ensure_witness(key)?;
        }
        let target = self.transactions.last_mut().unwrap_or(&mut self.writes);
        target.insert(key.to_vec(), None);
        Ok(())
    }
}

impl ManagedStateAccess for ProofState {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        ManagedState::get(self, key).map_err(state_access_error)
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), StateAccessError> {
        ManagedState::set(self, key, value).map_err(state_access_error)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), StateAccessError> {
        ManagedState::delete(self, key).map_err(state_access_error)
    }

    fn begin_transaction(&mut self) -> Result<(), StateAccessError> {
        Self::begin_transaction(self);
        Ok(())
    }

    fn commit_transaction(&mut self) -> Result<(), StateAccessError> {
        Self::commit_transaction(self).map_err(|_| StateAccessError::Backend)
    }

    fn rollback_transaction(&mut self) -> Result<(), StateAccessError> {
        Self::rollback_transaction(self).map_err(|_| StateAccessError::Backend)
    }
}

fn state_access_error(error: StateError) -> StateAccessError {
    match error {
        StateError::MissingWitness => StateAccessError::MissingWitness,
        StateError::InvalidProof => StateAccessError::InvalidProof,
        _ => StateAccessError::Backend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_root_changes_only_after_transaction_finishes() {
        assert_eq!(
            empty_state_root(),
            service_runtime_core::EMPTY_STATE_ROOT_V1
        );
        let base = FullState::from_pairs([(b"alice".as_slice(), b"100".as_slice())]).unwrap();
        let root = base.root();
        let mut tx = StateTransaction::new(base.clone());
        ManagedState::set(&mut tx, b"alice", b"90").unwrap();
        ManagedState::set(&mut tx, b"bob", b"60").unwrap();
        assert_eq!(base.root(), root);
        let (next, diff) = tx.finish().unwrap();
        assert_ne!(next.root(), root);
        assert_eq!(diff.changes.len(), 2);
        assert_eq!(next.get(b"alice").unwrap(), Some(b"90".to_vec()));
    }

    #[test]
    fn proof_round_trip_verifies_same_reads_and_root() {
        let base = FullState::from_pairs([
            (b"alice".as_slice(), b"100".as_slice()),
            (b"bob".as_slice(), b"50".as_slice()),
            (b"carol".as_slice(), b"20".as_slice()),
        ])
        .unwrap();
        let proof = base.proof_for(&[b"alice", b"bob"]).unwrap();
        let proof_nodes: Vec<Vec<u8>> = proof.clone().into_nodes().into_iter().collect();
        let proof_bytes = proof_nodes.clone();
        let mut verifier =
            ProofState::from_proof(base.root(), StorageProof::new(proof_bytes)).unwrap();
        assert_eq!(
            ManagedState::get(&mut verifier, b"alice").unwrap(),
            Some(b"100".to_vec())
        );
        assert_eq!(
            ManagedState::get(&mut verifier, b"bob").unwrap(),
            Some(b"50".to_vec())
        );
        ManagedState::set(&mut verifier, b"alice", b"90").unwrap();
        ManagedState::set(&mut verifier, b"bob", b"60").unwrap();
        let (root, _) = verifier.finish().unwrap();
        let host = base
            .apply_diff(&StateDiffV1 {
                changes: vec![
                    StateChangeV1 {
                        key: b"alice".to_vec(),
                        value: Some(b"90".to_vec()),
                    },
                    StateChangeV1 {
                        key: b"bob".to_vec(),
                        value: Some(b"60".to_vec()),
                    },
                ],
            })
            .unwrap();
        let diff_hash = service_runtime_core::state_delta_hash(&StateDiffV1 {
            changes: vec![
                StateChangeV1 {
                    key: b"alice".to_vec(),
                    value: Some(b"90".to_vec()),
                },
                StateChangeV1 {
                    key: b"bob".to_vec(),
                    value: Some(b"60".to_vec()),
                },
            ],
        })
        .unwrap();
        let proof_hash = service_runtime_core::blake2_256(
            &service_runtime_core::ManagedStateWitnessV1 {
                version: 1,
                parent_root: base.root(),
                storage_proof: proof_nodes,
            }
            .encode()
            .unwrap(),
        );
        assert_eq!(
            hex::encode(base.root()),
            "7ff64af161e33237740a218cddd8306f219e23ae4322ba1f0148a0de1cf10aec"
        );
        assert_eq!(
            hex::encode(diff_hash),
            "deb58561622c5e82653c393519561c349f7df2375e05fc16032b2c45717c445c"
        );
        assert_eq!(
            hex::encode(proof_hash),
            "55b15c7a56d6ecbb3d86cdd4af570921e4ca5bf853c1489bc07eae16b885f05a"
        );
        assert_eq!(
            hex::encode(host.root()),
            "38650a1401ad328caca70e927e88807a101efed1a8c0239d4b0988491e84dcfe"
        );
        assert_eq!(root, host.root());
    }

    #[test]
    fn missing_witness_is_not_a_non_inclusion_result() {
        let base = FullState::from_pairs([
            (b"alice".as_slice(), b"100".as_slice()),
            (b"bob".as_slice(), b"50".as_slice()),
            (b"carol".as_slice(), b"20".as_slice()),
        ])
        .unwrap();
        assert!(matches!(
            ProofState::from_proof(base.root(), StorageProof::new(Vec::new())),
            Err(StateError::MissingWitness)
        ));
    }

    #[test]
    fn tampered_proof_and_wrong_parent_root_fail() {
        let base = FullState::from_pairs([
            (b"alice".as_slice(), b"100".as_slice()),
            (b"bob".as_slice(), b"50".as_slice()),
        ])
        .unwrap();
        let mut nodes: Vec<Vec<u8>> = base
            .proof_for(&[b"alice"])
            .unwrap()
            .into_nodes()
            .into_iter()
            .collect();
        nodes[0][0] ^= 1;
        assert!(ProofState::from_proof(base.root(), StorageProof::new(nodes)).is_err());
        let proof = base.proof_for(&[b"alice"]).unwrap();
        assert!(ProofState::from_proof([8; 32], proof).is_err());
    }
}
