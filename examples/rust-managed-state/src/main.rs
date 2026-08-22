use service_runtime_core::{
    ExecutionContext, RuntimeRefineInputV1, ServiceApplication, ServiceKeyV1, StateAccessError,
    StateAccessPlanV1, StateChangeV1, StateDiffV1,
};
use service_runtime_guest::refine_v2;
use service_runtime_host::{FullStateProvider, ServiceStateProvider};
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
    let service = ServiceKeyV1::new([9; 32]);
    let state = FullState::empty();
    let mut provider = FullStateProvider::default();
    let parent_root = provider.insert(service, state.clone());
    let plan = StateAccessPlanV1::for_public([b"counter".as_slice()]).unwrap();
    let witness = provider.build_witness(service, parent_root, &plan).unwrap();
    let input = RuntimeRefineInputV1 {
        version: 1,
        managed_state: witness,
        actions: vec![b"increment".to_vec()],
    };
    let output = refine_v2(&Counter, &input).expect("counter transition");
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
    provider
        .apply_recovery(service, &output)
        .expect("provider recovery");
    assert_eq!(provider.current_root(service).unwrap(), output.new_root);
    assert_eq!(
        provider
            .open(service, output.new_root)
            .unwrap()
            .get(b"counter")
            .unwrap(),
        Some(vec![1])
    );
    println!(
        "authenticated Rust runtime transition and provider recovery: {:?}",
        output.new_root
    );
}
