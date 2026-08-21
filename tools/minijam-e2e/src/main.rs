#![feature(generic_const_exprs)]
#![allow(incomplete_features)]

use std::{collections::BTreeMap, env, fs, sync::Arc};

use jam_codec::Encode;
use jambda_minijam_executive::{
    system_service_genesis_state, MiniJamExecutive, SystemServiceGenesisConfig, SYSTEM_SERVICE_ID,
};
use jambda_refine::{compute_work_report, ImportProofBundle, WorkReportInput};
use jambda_state_backend::StateBackend;
use jamscript_crypto::SR25519_CONTEXT;
use jamscript_protocol::SignedActionV1;
use jamscript_runtime_core::{state_key, NONCE_SCHEMA_V1};
use jp_core_primitives::{
    blake2b,
    crypto::OpaqueHash,
    simple::{ByteSequence, TimeSlot},
    spec::TinySpec,
    state::{column, ColumnFamily, StateKey, StoreChange, StoreKey},
    traits::DataBase as CoreDataBase,
    types::ServiceInfo,
    work::{PreimagesLookups, RefineContext, WorkExecResult, WorkItem, WorkPackage},
};
use jp_vm_engine::InnerEngine;
use jp_vm_interp::InterpBackend;
use minijam_jamcore_api::{
    MiniJamExecutionInput, MiniJamExecutionOutput, ProtocolStateReader, StateError,
};
use minijam_protocol::{StateOperation, PROTOCOL_VERSION_V1};
use schnorrkel::{context::signing_context, ExpansionMode, MiniSecretKey};

const SERVICE_ID: u32 = 1_000;
const MAX_BLOCK_GAS: u64 = 20_000_000;
const ITEM_GAS: u64 = 5_000_000;
const NATIVE_ERROR_BASE: u32 = 0x8000_0000;

#[derive(Default)]
struct TestState(BTreeMap<[u8; 31], Vec<u8>>);

impl TestState {
    fn from_state(entries: Vec<(StateKey, Vec<u8>)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(key, value)| (key.0, value))
                .collect(),
        )
    }

    fn apply(&mut self, output: &MiniJamExecutionOutput) {
        for change in &output.ordered_changes {
            match change.operation {
                StateOperation::Upsert | StateOperation::Update => {
                    self.0
                        .insert(change.key, change.value.clone().unwrap().into_inner());
                }
                StateOperation::Remove => {
                    self.0.remove(&change.key);
                }
            }
        }
    }

    fn nonce(&self, sender: &[u8; 32]) -> Option<u64> {
        let key = state_key(SERVICE_ID, NONCE_SCHEMA_V1, sender);
        let storage_key =
            StoreKey::new_service_storage_key(&SERVICE_ID, &ByteSequence::from(key.to_vec()))
                .to_state_key()
                .0;
        self.0
            .get(&storage_key)
            .map(|value| u64::from_le_bytes(value.as_slice().try_into().unwrap()))
    }

    fn score(&self, sender: &[u8; 32]) -> Option<u64> {
        let key = state_key(SERVICE_ID, b"best-score/v1", sender);
        let storage_key =
            StoreKey::new_service_storage_key(&SERVICE_ID, &ByteSequence::from(key.to_vec()))
                .to_state_key()
                .0;
        self.0
            .get(&storage_key)
            .map(|value| u64::from_le_bytes(value.as_slice().try_into().unwrap()))
    }
}

impl ProtocolStateReader for TestState {
    fn get(&self, key: &[u8; 31]) -> Result<Option<Vec<u8>>, StateError> {
        Ok(self.0.get(key).cloned())
    }
}

struct MiniJamDb<'a> {
    state: &'a dyn ProtocolStateReader,
}

impl<'a> MiniJamDb<'a> {
    fn new(state: &'a dyn ProtocolStateReader) -> Self {
        Self { state }
    }
}

unsafe impl Sync for MiniJamDb<'_> {}

impl CoreDataBase for MiniJamDb<'_> {
    fn key_may_exist<K: AsRef<[u8]>>(&self, col: ColumnFamily, key: &K) -> bool {
        self.get(col, key).ok().flatten().is_some()
    }

    fn get<K: AsRef<[u8]>>(
        &self,
        col: ColumnFamily,
        key: &K,
    ) -> Result<Option<Vec<u8>>, jp_core_primitives::error::DataBaseError> {
        if col != column::COL_STATE || key.as_ref().len() != 31 {
            return Ok(None);
        }
        let mut state_key = [0u8; 31];
        state_key.copy_from_slice(key.as_ref());
        self.state.get(&state_key).map_err(|_| {
            jp_core_primitives::error::DataBaseError::Other("state read failed".into())
        })
    }

    fn del<K: AsRef<[u8]>>(
        &self,
        _: ColumnFamily,
        _: &K,
    ) -> Result<(), jp_core_primitives::error::DataBaseError> {
        Ok(())
    }

    fn multi_get<K: AsRef<[u8]>>(
        &self,
        keys: &[K],
        col: ColumnFamily,
    ) -> Result<Vec<Option<Vec<u8>>>, jp_core_primitives::error::DataBaseError> {
        keys.iter().map(|key| self.get(col, key)).collect()
    }

    fn put<K: AsRef<[u8]>>(
        &self,
        _: ColumnFamily,
        _: &K,
        _: Box<[u8]>,
    ) -> Result<(), jp_core_primitives::error::DataBaseError> {
        Ok(())
    }

    fn batch_write(
        &self,
        _: &[StoreChange],
    ) -> Result<(), jp_core_primitives::error::DataBaseError> {
        Ok(())
    }

    fn batch_write_cf<K: AsRef<[u8]>>(
        &self,
        _: ColumnFamily,
        _: &[(K, Vec<u8>)],
    ) -> Result<(), jp_core_primitives::error::DataBaseError> {
        Ok(())
    }

    fn multi_seek_for_prev<F>(
        &self,
        _: ColumnFamily,
        keys: &[&StateKey],
        mut callback: F,
    ) -> Result<(), jp_core_primitives::error::DataBaseError>
    where
        F: FnMut(usize, Option<(&[u8], &[u8])>),
    {
        for index in 0..keys.len() {
            callback(index, None);
        }
        Ok(())
    }
}

fn install_service(state: &mut TestState, blob: &[u8]) -> OpaqueHash {
    let code_hash = OpaqueHash(blake2b(blob));
    let mut info = ServiceInfo::new(
        code_hash,
        ITEM_GAS,
        0,
        0,
        TimeSlot(0),
        SYSTEM_SERVICE_ID,
        blob.len() as u64,
    );
    info.balance = info.balance.max(1_000_000_000_000);
    state.0.extend([
        (
            StoreKey::new_service_info_key(&SERVICE_ID).to_state_key().0,
            info.encode(),
        ),
        (
            StoreKey::new_preimage_key(&SERVICE_ID, &code_hash)
                .to_state_key()
                .0,
            blob.to_vec(),
        ),
        (
            StoreKey::new_service_lookups_key(&SERVICE_ID, &code_hash, blob.len() as u32)
                .to_state_key()
                .0,
            PreimagesLookups(vec![TimeSlot(0)]).encode(),
        ),
    ]);
    code_hash
}

fn action(
    seed: u8,
    nonce: u64,
    valid_until: u64,
    selector: [u8; 8],
    payload: Vec<u8>,
) -> (SignedActionV1, [u8; 32]) {
    let keypair = MiniSecretKey::from_bytes(&[seed; 32])
        .unwrap()
        .expand_to_keypair(ExpansionMode::Ed25519);
    let mut action = SignedActionV1::unsigned(
        [0; 32],
        SERVICE_ID,
        selector,
        keypair.public.to_bytes(),
        nonce,
        valid_until,
        payload,
    )
    .unwrap();
    let signature = keypair.sign(signing_context(SR25519_CONTEXT).bytes(&action.signing_digest()));
    action.signature = signature.to_bytes().to_vec();
    (action, keypair.public.to_bytes())
}

fn work_input(code_hash: OpaqueHash, payloads: Vec<Vec<u8>>, sequence: u8) -> WorkReportInput {
    let item_count = payloads.len();
    let package = WorkPackage {
        auth_code_host: SYSTEM_SERVICE_ID,
        auth_code_hash: OpaqueHash([0; 32]),
        context: RefineContext {
            anchor: OpaqueHash([sequence; 32]),
            state_root: OpaqueHash([0; 32]),
            beefy_root: OpaqueHash([0; 32]),
            lookup_anchor: OpaqueHash([0; 32]),
            lookup_anchor_slot: TimeSlot(0),
            prerequisites: Vec::new(),
        },
        authorization: ByteSequence::from(Vec::new()),
        authorizer_config: ByteSequence::from(Vec::new()),
        items: payloads
            .into_iter()
            .map(|payload| WorkItem {
                service: SERVICE_ID,
                code_hash,
                refine_gas_limit: ITEM_GAS,
                accumulate_gas_limit: ITEM_GAS,
                export_count: 0,
                payload: ByteSequence::from(payload),
                import_segments: Vec::new(),
                extrinsic: Vec::new(),
            })
            .collect(),
    };
    WorkReportInput {
        core_index: 0,
        work_package: Arc::new(package),
        external_data: Arc::new(vec![Vec::new(); item_count]),
        import_segments: Arc::new(vec![Vec::new(); item_count]),
        import_proofs: ImportProofBundle::default(),
    }
}

fn new_state() -> TestState {
    TestState::from_state(
        system_service_genesis_state(SystemServiceGenesisConfig {
            code_blob: include_bytes!("../../../../minijam-client/artifacts/system-service.blob")
                .to_vec(),
            initial_balance: 1_000_000_000_000,
            min_item_gas: 1,
            min_memo_gas: 1,
            deposit_offset: 0,
            genesis_slot: 0,
            parent_service: SYSTEM_SERVICE_ID,
        })
        .unwrap(),
    )
}

fn execute_batch(state: &mut TestState, code_hash: OpaqueHash, actions: Vec<Vec<u8>>, slot: u32) {
    let action_count = actions.len();
    let summary = state_summary(state, &actions);
    let mut report_bytes = Vec::with_capacity(actions.len());
    let mut result_summaries = Vec::with_capacity(actions.len());
    for (report_index, action) in actions.into_iter().enumerate() {
        let action_summary = state_summary(state, std::slice::from_ref(&action));
        let (encoded, result_summary) = {
            let mut backend = StateBackend::<TinySpec, _>::new_tiny(MiniJamDb::new(state));
            backend.load_tiny_from_db().unwrap();
            let report = compute_work_report::<
                TinySpec,
                MiniJamDb<'_>,
                StateBackend<TinySpec, MiniJamDb<'_>>,
                InterpBackend,
                InnerEngine<InterpBackend>,
            >(
                &backend,
                work_input(code_hash, vec![action], slot as u8),
                InterpBackend,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "MiniJAM refine failed: slot={slot}, report={report_index}, error={error:?}, state={action_summary}"
                )
            });
            let result_summary = report
                .report
                .results
                .iter()
                .enumerate()
                .map(|(index, result)| match &result.result {
                    WorkExecResult::Ok(payload) => {
                        match jamscript_runtime_core::decode_refined_action(payload.as_ref()) {
                            Ok(refined) => format!(
                                "item={index},sender={:?},nonce={},result={:?}",
                                refined.sender, refined.nonce, refined.result
                            ),
                            Err(error) => format!("item={index},error_payload={error:?}"),
                        }
                    }
                    result => format!("item={index},result={result:?}"),
                })
                .collect::<Vec<_>>()
                .join("; ");
            (report.report.encode(), result_summary)
        };
        report_bytes.push(encoded);
        result_summaries.push(format!("report={report_index}, {result_summary}"));
    }
    let reports = report_bytes
        .into_iter()
        .map(|bytes| bytes.try_into().unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let output = MiniJamExecutive
        .execute(
            MiniJamExecutionInput {
                protocol_version: PROTOCOL_VERSION_V1,
                slot,
                parent_hash: [0; 32],
                parent_state_root: [0; 32],
                entropy: [0; 32],
                reports,
                preimages: Default::default(),
                system_ops: Default::default(),
                max_gas: MAX_BLOCK_GAS,
            },
            state,
        )
        .unwrap_or_else(|error| {
            panic!(
                "MiniJAM accumulate failed: slot={slot}, actions={}, error={error:?}, state={summary}, results=[{}]",
                action_count,
                result_summaries.join("; "),
            )
        });
    state.apply(&output);
}

fn refine_native_failure(
    state: &TestState,
    code_hash: OpaqueHash,
    action: Vec<u8>,
    slot: u32,
    expected_native_status: u32,
) {
    let summary = state_summary(state, std::slice::from_ref(&action));
    let mut backend = StateBackend::<TinySpec, _>::new_tiny(MiniJamDb::new(state));
    backend.load_tiny_from_db().unwrap();
    let report = compute_work_report::<
        TinySpec,
        MiniJamDb<'_>,
        StateBackend<TinySpec, MiniJamDb<'_>>,
        InterpBackend,
        InnerEngine<InterpBackend>,
    >(
        &backend,
        work_input(code_hash, vec![action], slot as u8),
        InterpBackend,
    )
    .unwrap_or_else(|error| {
        panic!(
            "MiniJAM refine failed: slot={slot}, error={error:?}, state={}",
            summary,
        )
    });
    match &report.report.results[0].result {
        WorkExecResult::Ok(payload) => {
            let expected = (NATIVE_ERROR_BASE | expected_native_status).to_le_bytes();
            let actual: &[u8] = payload.as_ref();
            assert_eq!(
                actual,
                expected.as_slice(),
                "native error payload: slot={slot}, expected={expected:?}, actual={actual:?}, state={summary}"
            );
        }
        result => panic!(
            "native failure was not an Ok(error payload): slot={slot}, result={result:?}, state={}",
            summary,
        ),
    }
}

fn state_summary(state: &TestState, actions: &[Vec<u8>]) -> String {
    let items = actions
        .iter()
        .enumerate()
        .map(|(index, encoded)| match SignedActionV1::decode(encoded) {
            Ok(action) if action.public_key.len() == 32 => {
                let mut sender = [0u8; 32];
                sender.copy_from_slice(&action.public_key);
                format!(
                    "item={index},sender={sender:?},nonce={},state_nonce={:?},score={:?},result={:?}",
                    action.nonce,
                    state.nonce(&sender),
                    state.score(&sender),
                    action.payload.get(..4),
                )
            }
            Ok(action) => format!(
                "item={index},sender_len={},nonce={},result={:?}",
                action.public_key.len(),
                action.nonce,
                action.payload.get(..4),
            ),
            Err(error) => format!("item={index},decode_error={error:?}"),
        })
        .collect::<Vec<_>>();
    format!(
        "service_id={SERVICE_ID},entries={},[{}]",
        state.0.len(),
        items.join("; ")
    )
}

fn game_run(health: u32, steps: &[(u8, u8)]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(12 + steps.len() * 4);
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&health.to_le_bytes());
    payload.extend_from_slice(&(steps.len() as u32).to_le_bytes());
    for &(opcode, amount) in steps {
        payload.extend_from_slice(&[opcode, amount, 0, 0]);
    }
    payload
}

fn signed_game(seed: u8, nonce: u64, valid_until: u64, run: Vec<u8>) -> (SignedActionV1, [u8; 32]) {
    let mut payload = Vec::with_capacity(4 + run.len());
    payload.extend_from_slice(&(run.len() as u32).to_le_bytes());
    payload.extend_from_slice(&run);
    action(
        seed,
        nonce,
        valid_until,
        jamscript_ir::action_selector("submitRun"),
        payload,
    )
}

fn run_counter(blob: &[u8]) {
    let mut state = new_state();
    let code_hash = install_service(&mut state, blob);
    let selector = jamscript_ir::action_selector("increment");
    let (_, sender) = action(7, 0, 10, selector, 7u64.to_le_bytes().to_vec());
    for (slot, nonce, valid_until, expected) in [
        (1u32, 0u64, 10u64, 1u64),
        (2, 0, 10, 1),
        (3, 1, 10, 2),
        (11, 2, 10, 2),
    ] {
        let (signed, action_sender) =
            action(7, nonce, valid_until, selector, 7u64.to_le_bytes().to_vec());
        assert_eq!(sender, action_sender);
        execute_batch(&mut state, code_hash, vec![signed.encode().unwrap()], slot);
        assert_eq!(state.nonce(&sender), Some(expected));
    }
}

fn run_game(blob: &[u8]) {
    let selector = jamscript_ir::action_selector("submitRun");
    let valid_100 = game_run(80, &[(1, 20)]);
    let valid_80 = game_run(80, &[]);
    let valid_150 = game_run(100, &[(1, 50)]);
    let mut state = new_state();
    let code_hash = install_service(&mut state, blob);
    let (signed, alice) = signed_game(7, 0, 10, valid_100.clone());
    execute_batch(&mut state, code_hash, vec![signed.encode().unwrap()], 1);
    assert_eq!(state.score(&alice), Some(100));
    assert_eq!(state.nonce(&alice), Some(1));

    let (mut tampered, _) = signed_game(7, 1, 10, valid_100.clone());
    tampered.payload[12] ^= 1;
    execute_batch(&mut state, code_hash, vec![tampered.encode().unwrap()], 2);
    assert_eq!(state.score(&alice), Some(100));
    assert_eq!(state.nonce(&alice), Some(1));

    let (invalid, _) = signed_game(7, 1, 10, game_run(80, &[(9, 20)]));
    refine_native_failure(&state, code_hash, invalid.encode().unwrap(), 2, 9);
    assert_eq!(state.score(&alice), Some(100));
    assert_eq!(state.nonce(&alice), Some(1));

    let mut trailing_run = valid_100.clone();
    trailing_run.push(0);
    let (trailing, _) = signed_game(7, 1, 10, trailing_run);
    refine_native_failure(&state, code_hash, trailing.encode().unwrap(), 2, 5);
    assert_eq!(state.score(&alice), Some(100));
    assert_eq!(state.nonce(&alice), Some(1));

    let (replay, _) = signed_game(7, 0, 10, valid_100.clone());
    execute_batch(&mut state, code_hash, vec![replay.encode().unwrap()], 3);
    assert_eq!(state.score(&alice), Some(100));
    assert_eq!(state.nonce(&alice), Some(1));

    let (lower, _) = signed_game(7, 1, 10, valid_80);
    execute_batch(&mut state, code_hash, vec![lower.encode().unwrap()], 4);
    assert_eq!(state.score(&alice), Some(100));
    assert_eq!(state.nonce(&alice), Some(2));

    let (higher, _) = signed_game(7, 2, 10, valid_150.clone());
    execute_batch(&mut state, code_hash, vec![higher.encode().unwrap()], 5);
    assert_eq!(state.score(&alice), Some(150));
    assert_eq!(state.nonce(&alice), Some(3));

    let (expired, _) = signed_game(7, 3, 10, valid_150);
    execute_batch(&mut state, code_hash, vec![expired.encode().unwrap()], 11);
    assert_eq!(state.score(&alice), Some(150));
    assert_eq!(state.nonce(&alice), Some(3));

    let mut batch_state = new_state();
    let batch_hash = install_service(&mut batch_state, blob);
    let mut batch = Vec::new();
    let mut senders = Vec::new();
    for (seed, score) in [(1u8, 40u8), (2, 50), (3, 60)] {
        let (item, sender) = signed_game(seed, 0, 10, game_run(40, &[(1, score)]));
        batch.push(item.encode().unwrap());
        senders.push(sender);
    }
    execute_batch(&mut batch_state, batch_hash, batch, 1);
    for sender in senders {
        assert_eq!(batch_state.nonce(&sender), Some(1));
    }

    let mut isolation_state = new_state();
    let isolation_hash = install_service(&mut isolation_state, blob);
    let (good_a, sender_a) = signed_game(4, 0, 10, game_run(40, &[(1, 20)]));
    let (bad_b, sender_b) = signed_game(5, 0, 10, game_run(40, &[(8, 20)]));
    let (good_c, sender_c) = signed_game(6, 0, 10, game_run(40, &[(1, 30)]));
    execute_batch(
        &mut isolation_state,
        isolation_hash,
        vec![
            good_a.encode().unwrap(),
            bad_b.encode().unwrap(),
            good_c.encode().unwrap(),
        ],
        1,
    );
    assert_eq!(isolation_state.nonce(&sender_a), Some(1));
    assert_eq!(isolation_state.nonce(&sender_b), None);
    assert_eq!(isolation_state.nonce(&sender_c), Some(1));

    let mut sequential_state = new_state();
    let sequential_hash = install_service(&mut sequential_state, blob);
    let (first, sequential_sender) = signed_game(8, 0, 10, game_run(80, &[(1, 20)]));
    let (second, _) = signed_game(8, 1, 10, game_run(100, &[(1, 50)]));
    execute_batch(
        &mut sequential_state,
        sequential_hash,
        vec![first.encode().unwrap(), second.encode().unwrap()],
        1,
    );
    assert_eq!(sequential_state.nonce(&sequential_sender), Some(2));
    assert_eq!(sequential_state.score(&sequential_sender), Some(150));
    let _ = selector;
}

fn main() {
    let mut args = env::args().skip(1);
    let counter_path = args
        .next()
        .expect("usage: jamscript-minijam-e2e <counter.blob> <game.blob>");
    let game_path = args
        .next()
        .expect("usage: jamscript-minijam-e2e <counter.blob> <game.blob>");
    let counter = fs::read(counter_path).expect("read counter service blob");
    let game = fs::read(game_path).expect("read game service blob");
    run_counter(&counter);
    println!(
        "MiniJAM wallet E2E passed: valid nonce, replay rejection, next nonce, expiry rejection."
    );
    run_game(&game);
    println!("MiniJAM M4 game E2E passed: canonical bytes, native replay, tamper/native failure isolation, max state, query ABI path, batch nonce semantics.");
}
