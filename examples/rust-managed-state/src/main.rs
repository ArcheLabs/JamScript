use service_runtime_core::{
    ExecutionContext, ManagedStateCommitmentV1, ServiceApplication, ServiceKeyV1, StateAccessError,
    StateChangeV1, StateDiffV1, MANAGED_STATE_COMMITMENT_KEY_V1,
};
use service_runtime_host::{
    FinalizedContextV1, FinalizedManagedStateSource, FullStateProvider, ManagedStateWorkBuilder,
    MaterializedServiceStateProvider,
};
use service_runtime_state::FullState;

struct Counter;

struct FinalizedSource {
    context: FinalizedContextV1,
    commitment: Vec<u8>,
}

impl FinalizedManagedStateSource for FinalizedSource {
    type Error = ();

    fn finalized_context(&mut self) -> Result<FinalizedContextV1, Self::Error> {
        Ok(self.context)
    }

    fn service_storage_at(
        &mut self,
        context: &FinalizedContextV1,
        _service: ServiceKeyV1,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        if *context != self.context || key != MANAGED_STATE_COMMITMENT_KEY_V1 {
            return Err(());
        }
        Ok(Some(self.commitment.clone()))
    }
}

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
    let mut source = FinalizedSource {
        context: FinalizedContextV1 {
            block_hash: [1; 32],
            state_root: [2; 32],
            slot: 3,
        },
        commitment: ManagedStateCommitmentV1::new(parent_root).encode().to_vec(),
    };
    let built = ManagedStateWorkBuilder::new(&mut source, &provider)
        .build_one(service, &Counter, b"increment".to_vec())
        .expect("counter Work build");
    let output = built.predicted_output;
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
    assert_eq!(
        provider.materialized_root(service).unwrap(),
        output.new_root
    );
    assert_eq!(
        provider
            .open(service, output.new_root)
            .unwrap()
            .get(b"counter")
            .unwrap(),
        Some(vec![1])
    );
    println!(
        "finalized-root Work build and provider recovery: {:?}",
        output.new_root
    );
}
