use service_runtime_core::{
    ExecutionContext, ManagedStateAccess, RuntimeRefineInputV1, ServiceApplication,
    StateAccessError, StateRoot,
};
use service_runtime_guest::{refine, GuestError, GuestState};
use std::collections::BTreeMap;

struct Counter;

impl ServiceApplication for Counter {
    type Error = StateAccessError;

    fn execute(
        &self,
        context: &mut ExecutionContext<'_>,
        _input: &[u8],
    ) -> Result<(), Self::Error> {
        let current = context.state().get(b"counter")?.unwrap_or_default();
        let value = current.first().copied().unwrap_or(0).saturating_add(1);
        context.state().set(b"counter", &[value])
    }
}

struct MemoryState {
    root: StateRoot,
    values: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl ManagedStateAccess for MemoryState {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, StateAccessError> {
        Ok(self.values.get(key).cloned())
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), StateAccessError> {
        self.values.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), StateAccessError> {
        self.values.remove(key);
        Ok(())
    }
}

impl GuestState for MemoryState {
    fn parent_root(&self) -> StateRoot {
        self.root
    }

    fn finish_root(&mut self) -> Result<StateRoot, GuestError> {
        self.root[0] = self
            .values
            .get(b"counter".as_slice())
            .and_then(|value| value.first())
            .copied()
            .unwrap_or(0);
        Ok(self.root)
    }
}

fn main() {
    let mut state = MemoryState {
        root: [0; 32],
        values: BTreeMap::new(),
    };
    let input = RuntimeRefineInputV1 {
        version: 1,
        managed_state: service_runtime_core::ManagedStateWitnessV1 {
            version: 1,
            parent_root: [0; 32],
            storage_proof: Vec::new(),
        },
        actions: vec![b"increment".to_vec()],
    };
    let output = refine(&Counter, &mut state, &input).expect("counter transition");
    assert_eq!(output.new_root[0], 1);
    println!("direct Rust runtime transition: {:?}", output.new_root);
}
