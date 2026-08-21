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
const REFINE_GAS: u64 = 10_000_000;
const ACCUMULATE_GAS: u64 = 10_000_000;

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
        ACCUMULATE_GAS,
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

fn action(seed: u8, nonce: u64, valid_until: u64) -> (SignedActionV1, [u8; 32]) {
    let keypair = MiniSecretKey::from_bytes(&[seed; 32])
        .unwrap()
        .expand_to_keypair(ExpansionMode::Ed25519);
    let mut action = SignedActionV1::unsigned(
        [0; 32],
        SERVICE_ID,
        jamscript_ir::action_selector("increment"),
        keypair.public.to_bytes(),
        nonce,
        valid_until,
        7u64.to_le_bytes().to_vec(),
    )
    .unwrap();
    let signature = keypair.sign(signing_context(SR25519_CONTEXT).bytes(&action.signing_digest()));
    action.signature = signature.to_bytes().to_vec();
    (action, keypair.public.to_bytes())
}

fn work_input(code_hash: OpaqueHash, payload: Vec<u8>, sequence: u8) -> WorkReportInput {
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
        items: vec![WorkItem {
            service: SERVICE_ID,
            code_hash,
            refine_gas_limit: REFINE_GAS,
            accumulate_gas_limit: ACCUMULATE_GAS,
            export_count: 0,
            payload: ByteSequence::from(payload),
            import_segments: Vec::new(),
            extrinsic: Vec::new(),
        }],
    };
    WorkReportInput {
        core_index: 0,
        work_package: Arc::new(package),
        external_data: Arc::new(vec![Vec::new()]),
        import_segments: Arc::new(vec![Vec::new()]),
        import_proofs: ImportProofBundle::default(),
    }
}

fn main() {
    let blob_path = env::args()
        .nth(1)
        .expect("usage: jamscript-minijam-e2e <service.blob>");
    let blob = fs::read(blob_path).expect("read service blob");
    let mut state = TestState::from_state(
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
    );
    let code_hash = install_service(&mut state, &blob);
    let (_, sender) = action(7, 0, 10);

    for (slot, nonce, valid_until, expected) in [
        (1u32, 0u64, 10u64, 1u64),
        (2, 0, 10, 1),
        (3, 1, 10, 2),
        (11, 2, 10, 2),
    ] {
        let (signed, action_sender) = action(7, nonce, valid_until);
        assert_eq!(sender, action_sender);
        let mut backend = StateBackend::<TinySpec, _>::new_tiny(MiniJamDb::new(&state));
        backend.load_tiny_from_db().unwrap();
        let report = compute_work_report::<
            TinySpec,
            MiniJamDb<'_>,
            StateBackend<TinySpec, MiniJamDb<'_>>,
            InterpBackend,
            InnerEngine<InterpBackend>,
        >(
            &backend,
            work_input(code_hash, signed.encode().unwrap(), slot as u8),
            InterpBackend::default(),
        )
        .expect("Refine execution");
        assert!(matches!(
            report.report.results[0].result,
            WorkExecResult::Ok(_)
        ));
        let output = MiniJamExecutive
            .execute(
                MiniJamExecutionInput {
                    protocol_version: PROTOCOL_VERSION_V1,
                    slot,
                    parent_hash: [0; 32],
                    parent_state_root: [0; 32],
                    entropy: [0; 32],
                    reports: vec![report.report.encode().try_into().unwrap()]
                        .try_into()
                        .unwrap(),
                    preimages: Default::default(),
                    system_ops: Default::default(),
                    max_gas: ACCUMULATE_GAS,
                },
                &state,
            )
            .expect("Accumulate execution");
        state.apply(&output);
        assert_eq!(state.nonce(&sender), Some(expected));
    }
    println!(
        "MiniJAM wallet E2E passed: valid nonce, replay rejection, next nonce, expiry rejection."
    );
}
