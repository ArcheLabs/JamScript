use service_runtime_core::{
    ExecutionContext, ManagedStateWitnessV1, RuntimeRefineInputV1, ServiceApplication,
    StateAccessError, StateChangeV1, StateDiffV1,
};
use service_runtime_guest::refine;
use service_runtime_state::FullState;

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

fn main() {
    let state = FullState::empty();
    let parent_root = state.root();
    let proof = state.proof_for(&[b"counter"]).unwrap();
    let proof_nodes = proof.into_nodes().into_iter().collect();
    let input = RuntimeRefineInputV1 {
        version: 1,
        managed_state: ManagedStateWitnessV1 {
            version: 1,
            parent_root,
            storage_proof: proof_nodes,
        },
        actions: vec![b"increment".to_vec()],
    };
    let output = refine(&Counter, &input).expect("counter transition");
    let expected = state
        .apply_diff(&StateDiffV1 {
            changes: vec![StateChangeV1 {
                key: b"counter".to_vec(),
                value: Some(vec![1]),
            }],
        })
        .unwrap();
    assert_eq!(output.parent_root, parent_root);
    assert_eq!(output.new_root, expected.root());
    assert_eq!(output.receipts.len(), 1);
    assert_eq!(
        output.receipts[0].status,
        service_runtime_core::ActionStatusV1::Applied
    );
    println!(
        "authenticated Rust runtime transition: {:?}",
        output.new_root
    );
}
