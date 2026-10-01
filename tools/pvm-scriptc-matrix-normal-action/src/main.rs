use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use jamscript_codec::{encode as encode_value, Value};
use jamscript_ir::{action_selector, FieldIr, TypeIr};
use jamscript_protocol::SignedActionV2;
use ownership_core::{Ownership, OwnershipKind};
use polkavm::{
    BackendKind, CallError, Config, Engine, Linker, MemoryAccessError, Module, ModuleConfig,
    ProgramBlob, ProgramCounter, Reg,
};
use service_runtime_core::{
    application_key_v1, decode_planner_need_state, ActionStatusV1, ManagedStateWitnessV1,
    RuntimeRefineInputV1, RuntimeRefineOutputV1, ServiceKeyV1, StateAccessPlanV1, StateRecoveryV1,
};
use service_runtime_state::FullState;
use std::{env, fs, path::PathBuf};

const SERVICE_KEY: [u8; 32] = [0x44; 32];
const LOCUS_SERVICE_KEY: [u8; 32] = [
    0x5b, 0x98, 0x2e, 0xff, 0xef, 0x6c, 0x8f, 0x72, 0x18, 0x52, 0x78, 0x33, 0x76, 0x23, 0x95, 0xb7,
    0xa8, 0x59, 0x60, 0x6b, 0x81, 0x51, 0x36, 0x38, 0xb5, 0x45, 0xfd, 0x48, 0x35, 0xe4, 0x7d, 0xa2,
];
const NETWORK_DOMAIN: [u8; 32] = [0; 32];
const GRANT_SCHEMA: &[u8] = b"test.controller-grant.v1";
const LOCUS_GRANT_SCHEMA: &[u8] = b"locus.controller-grant.v1";
// Match the production PvmApplication planner/refine budget exactly.
const PVM_GAS: i64 = 5_000_000;

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let artifact_path = args
        .next()
        .map(PathBuf::from)
        .context("usage: pvm-scriptc-matrix-normal-action <probe.pvm> [locus.pvm]")?;
    let locus_artifact_path = args.next().map(PathBuf::from);
    if args.next().is_some() {
        bail!("usage: pvm-scriptc-matrix-normal-action <probe.pvm> [locus.pvm]");
    }
    let artifact =
        fs::read(&artifact_path).with_context(|| format!("reading {}", artifact_path.display()))?;
    let artifact_digest = hex(&service_runtime_core::blake2_256(&artifact));
    let engine = make_engine()?;
    let master_key = SigningKey::from_bytes(&[1; 32]);
    let device_key = SigningKey::from_bytes(&[2; 32]);
    let master = Ownership::from_array(
        OwnershipKind::Ed25519Key,
        master_key.verifying_key().to_bytes(),
    )?;
    let device = Ownership::from_array(
        OwnershipKind::Ed25519Key,
        device_key.verifying_key().to_bytes(),
    )?;

    let direct = signed_action("directOwnership", &device_key, Vec::new())?;
    let direct_keys = plan_action(&engine, &artifact, &direct, FullState::empty())
        .context("Test A: direct Ed25519 SignedActionV2 planner")?;
    let direct_output = refine_action(
        &engine,
        &artifact,
        &direct,
        FullState::empty(),
        &direct_keys,
    )
    .context("Test A: direct Ed25519 SignedActionV2 refine")?;
    require_applied(&direct_output, "direct Ed25519 ownership")?;
    println!("ED25519_DIRECT_PLAN=PASS");
    println!("ED25519_DIRECT_ACTION=APPLIED");
    println!("SIGNED_ACTION_V2_ED25519_PVM=PASS");
    println!("MATRIX_ED25519_DIRECT_OWNERSHIP_PVM=PASS");

    let delegated_payload = encode_value(&TypeIr::Ownership, &Value::Ownership(master.clone()))?;
    let delegated = signed_action("delegatedOwnership", &device_key, delegated_payload)?;
    let grant_key = controller_grant_key(&master, &device)?;
    let active_grant_state = FullState::from_pairs([(&grant_key, &[1u8][..])])
        .map_err(|error| anyhow::anyhow!("building active grant state: {error:?}"))?;
    let delegated_keys = plan_action(&engine, &artifact, &delegated, active_grant_state.clone())
        .context("Test B: active delegated M→D ownership planner")?;
    let delegated_output = refine_action(
        &engine,
        &artifact,
        &delegated,
        active_grant_state,
        &delegated_keys,
    )
    .context("Test B: active delegated M→D ownership refine")?;
    require_applied(&delegated_output, "active delegated M→D ownership")?;
    println!("MATRIX_DELEGATED_OWNERSHIP_PVM=PASS");

    for (label, grant_state) in [
        ("missing", FullState::empty()),
        (
            "revoked",
            FullState::from_pairs([(&grant_key, &[0u8][..])])
                .map_err(|error| anyhow::anyhow!("building revoked grant state: {error:?}"))?,
        ),
    ] {
        let keys = plan_action(&engine, &artifact, &delegated, grant_state.clone())
            .with_context(|| format!("Test B: {label} delegated grant planner"))?;
        let output = refine_action(&engine, &artifact, &delegated, grant_state, &keys)
            .with_context(|| format!("Test B: {label} delegated grant refine"))?;
        require_abort(&output, 5001, label)?;
        println!(
            "MATRIX_{}_CONTROLLER=STRUCTURED_ABORT",
            label.to_ascii_uppercase()
        );
    }
    println!("MATRIX_UNAUTHORIZED_FAILURE_STRUCTURED=PASS");
    println!("MATRIX_UNAUTHORIZED_CONTROLLER=ABORT_5001");
    println!("MATRIX_REVOKED_CONTROLLER=ABORT_5001");

    if let Some(locus_artifact_path) = locus_artifact_path {
        let locus_artifact = fs::read(&locus_artifact_path)
            .with_context(|| format!("reading {}", locus_artifact_path.display()))?;
        run_locus_transfer(&engine, &locus_artifact, &master, &device, &device_key)?;
    }

    println!("UNCLASSIFIED_PVM_TRAP=0");
    println!("SERVICE_ARTIFACT_BLAKE2={artifact_digest}");
    println!("MATRIX_NORMAL_ACTION_PVM=PASS");
    Ok(())
}

fn signed_action(name: &str, key: &SigningKey, payload: Vec<u8>) -> Result<Vec<u8>> {
    signed_action_for(SERVICE_KEY, name, key, payload)
}

fn signed_action_for(
    service_key: [u8; 32],
    name: &str,
    key: &SigningKey,
    payload: Vec<u8>,
) -> Result<Vec<u8>> {
    let controller =
        Ownership::from_array(OwnershipKind::Ed25519Key, key.verifying_key().to_bytes())?;
    let mut action = SignedActionV2::unsigned(
        NETWORK_DOMAIN,
        ServiceKeyV1::new(service_key),
        action_selector(name),
        controller,
        None,
        0,
        100,
        payload,
    )?;
    action.authorization_proof = key.sign(&action.signing_message()?).to_bytes().to_vec();
    Ok(action.encode()?)
}

fn controller_grant_key(subject: &Ownership, controller: &Ownership) -> Result<Vec<u8>> {
    controller_grant_key_for(GRANT_SCHEMA, subject, controller)
}

fn controller_grant_key_for(
    schema: &[u8],
    subject: &Ownership,
    controller: &Ownership,
) -> Result<Vec<u8>> {
    let mut canonical_key = Vec::with_capacity(64);
    canonical_key.extend_from_slice(&subject.key()?);
    canonical_key.extend_from_slice(&controller.key()?);
    application_key_v1(schema, &canonical_key)
        .map_err(|error| anyhow::anyhow!("building controller grant key: {error:?}"))
}

fn run_locus_transfer(
    engine: &Engine,
    artifact: &[u8],
    subject: &Ownership,
    controller: &Ownership,
    controller_key: &SigningKey,
) -> Result<()> {
    // Run the same Locus action through the direct-controller path as a
    // control, then through the Matrix subject→device grant path.
    run_locus_transfer_case(
        engine,
        artifact,
        controller,
        controller,
        controller_key,
        "LOCUS_DIRECT_TRANSFER",
    )?;
    run_locus_transfer_case(
        engine,
        artifact,
        subject,
        controller,
        controller_key,
        "MATRIX_TRANSFER",
    )?;
    run_locus_create_pool(engine, artifact, subject, controller, controller_key)?;
    run_locus_swap(engine, artifact, subject, controller, controller_key)
}

fn run_locus_transfer_case(
    engine: &Engine,
    artifact: &[u8],
    subject: &Ownership,
    controller: &Ownership,
    controller_key: &SigningKey,
    marker: &str,
) -> Result<()> {
    const ASSET_ID: [u8; 32] = [0x73; 32];
    const TRANSFER_AMOUNT: u128 = 13;
    const INITIAL_SENDER_BALANCE: u128 = 1_000;
    const INITIAL_RECIPIENT_BALANCE: u128 = 100;

    let recipient_signing_key = SigningKey::from_bytes(&[3; 32]);
    let recipient = Ownership::from_array(
        OwnershipKind::Ed25519Key,
        recipient_signing_key.verifying_key().to_bytes(),
    )?;
    let subject_key = subject
        .key()
        .map_err(|error| anyhow::anyhow!("deriving Matrix subject ownership key: {error:?}"))?;
    let recipient_key = recipient
        .key()
        .map_err(|error| anyhow::anyhow!("deriving recipient ownership key: {error:?}"))?;
    let payload = encode_value(
        &TypeIr::Record {
            fields: vec![
                FieldIr {
                    name: "subject".into(),
                    ty: TypeIr::Ownership,
                },
                FieldIr {
                    name: "assetId".into(),
                    ty: TypeIr::FixedBytes { len: 32 },
                },
                FieldIr {
                    name: "to".into(),
                    ty: TypeIr::Ownership,
                },
                FieldIr {
                    name: "amount".into(),
                    ty: TypeIr::U128,
                },
            ],
        },
        &Value::Record(vec![
            ("subject".into(), Value::Ownership(subject.clone())),
            ("assetId".into(), Value::Bytes(ASSET_ID.to_vec())),
            ("to".into(), Value::Ownership(recipient.clone())),
            ("amount".into(), Value::Unsigned(TRANSFER_AMOUNT)),
        ]),
    )?;
    let signed = signed_action_for(LOCUS_SERVICE_KEY, "transfer", controller_key, payload)?;

    let grant_key = controller_grant_key_for(LOCUS_GRANT_SCHEMA, subject, controller)?;
    let asset_key = application_key_v1(b"locus.asset.v2", &ASSET_ID)
        .map_err(|error| anyhow::anyhow!("building Locus asset key: {error:?}"))?;
    let sender_balance_key = locus_balance_key(&ASSET_ID, &subject_key)?;
    let recipient_balance_key = locus_balance_key(&ASSET_ID, &recipient_key)?;
    let asset_value = encode_value(
        &TypeIr::Record {
            fields: vec![
                FieldIr {
                    name: "version".into(),
                    ty: TypeIr::U8,
                },
                FieldIr {
                    name: "issuer".into(),
                    ty: TypeIr::Ownership,
                },
                FieldIr {
                    name: "name".into(),
                    ty: TypeIr::Bytes { max: 64 },
                },
                FieldIr {
                    name: "symbol".into(),
                    ty: TypeIr::Bytes { max: 16 },
                },
                FieldIr {
                    name: "decimals".into(),
                    ty: TypeIr::U8,
                },
                FieldIr {
                    name: "totalSupply".into(),
                    ty: TypeIr::U128,
                },
            ],
        },
        &Value::Record(vec![
            ("version".into(), Value::Unsigned(2)),
            ("issuer".into(), Value::Ownership(subject.clone())),
            ("name".into(), Value::Bytes(b"Matrix probe asset".to_vec())),
            ("symbol".into(), Value::Bytes(b"MPRB".to_vec())),
            ("decimals".into(), Value::Unsigned(0)),
            ("totalSupply".into(), Value::Unsigned(2_000)),
        ]),
    )?;
    let sender_balance = INITIAL_SENDER_BALANCE.to_le_bytes();
    let recipient_balance = INITIAL_RECIPIENT_BALANCE.to_le_bytes();
    let mut initial_state = vec![
        (asset_key, asset_value),
        (sender_balance_key.clone(), sender_balance.to_vec()),
        (recipient_balance_key.clone(), recipient_balance.to_vec()),
    ];
    if subject != controller {
        initial_state.push((grant_key, vec![1u8]));
    }
    let state = FullState::from_pairs(
        initial_state
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice())),
    )
    .map_err(|error| anyhow::anyhow!("building Locus transfer state: {error:?}"))?;

    let keys = plan_action(engine, artifact, &signed, state.clone())
        .with_context(|| format!("Test C: {marker} planner"))?;
    println!("{marker}_PLAN=PASS");
    let output = refine_action(engine, artifact, &signed, state.clone(), &keys)
        .with_context(|| format!("Test C: {marker} refine"))?;
    require_applied(&output, marker)?;
    println!("{marker}_RECEIPT=APPLIED");

    if output.parent_root != state.root() {
        bail!("{marker} parent root does not match fixture state");
    }
    let recovery = StateRecoveryV1::decode(&output.recovery_payload)
        .map_err(|error| anyhow::anyhow!("decoding {marker} state recovery: {error:?}"))?;
    let next_state = state
        .apply_diff(&recovery.diff)
        .map_err(|error| anyhow::anyhow!("applying {marker} state diff: {error:?}"))?;
    let expected_sender = INITIAL_SENDER_BALANCE - TRANSFER_AMOUNT;
    let expected_recipient = INITIAL_RECIPIENT_BALANCE + TRANSFER_AMOUNT;
    let actual_sender = read_u128(&next_state, &sender_balance_key)?;
    let actual_recipient = read_u128(&next_state, &recipient_balance_key)?;
    if actual_sender != expected_sender || actual_recipient != expected_recipient {
        bail!(
            "{marker} balances mismatch: sender={actual_sender} expected={expected_sender} recipient={actual_recipient} expected={expected_recipient}"
        );
    }
    if next_state.root() != output.new_root {
        bail!("{marker} new root does not match canonical state");
    }
    println!("{marker}_STATE=PASS");
    println!("{marker}=PASS");
    if marker == "MATRIX_TRANSFER" {
        println!("MATRIX_TRANSFER_REAL_PVM=PASS");
    }
    Ok(())
}

fn locus_balance_key(asset_id: &[u8; 32], owner_key: &[u8; 32]) -> Result<Vec<u8>> {
    let mut canonical = Vec::with_capacity(64);
    canonical.extend_from_slice(asset_id);
    canonical.extend_from_slice(owner_key);
    application_key_v1(b"locus.balance.v2", &canonical)
        .map_err(|error| anyhow::anyhow!("building Locus balance key: {error:?}"))
}

fn run_locus_create_pool(
    engine: &Engine,
    artifact: &[u8],
    subject: &Ownership,
    controller: &Ownership,
    controller_signing_key: &SigningKey,
) -> Result<()> {
    const ASSET_A: [u8; 32] = [0x41; 32];
    const ASSET_B: [u8; 32] = [0x42; 32];
    const AMOUNT_A: u128 = 100;
    const AMOUNT_B: u128 = 400;
    const INITIAL_SHARES: u128 = 200;

    let subject_key = ownership_key(subject)?;
    let grant_key = controller_grant_key_for(LOCUS_GRANT_SCHEMA, subject, controller)?;
    let asset_a_key = application_key_v1(b"locus.asset.v2", &ASSET_A)
        .map_err(|error| anyhow::anyhow!("building asset A key: {error:?}"))?;
    let asset_b_key = application_key_v1(b"locus.asset.v2", &ASSET_B)
        .map_err(|error| anyhow::anyhow!("building asset B key: {error:?}"))?;
    let asset_value = encode_locus_asset_value(subject)?;
    let balance_a_key = locus_balance_key(&ASSET_A, &subject_key)?;
    let balance_b_key = locus_balance_key(&ASSET_B, &subject_key)?;
    let balance_a = 1_000u128.to_le_bytes();
    let balance_b = 2_000u128.to_le_bytes();
    let pool_key = locus_pool_key(&ASSET_A, &ASSET_B)?;

    let payload = encode_value(
        &TypeIr::Record {
            fields: vec![
                field("subject", TypeIr::Ownership),
                field("assetA", TypeIr::FixedBytes { len: 32 }),
                field("assetB", TypeIr::FixedBytes { len: 32 }),
                field("amountA", TypeIr::U128),
                field("amountB", TypeIr::U128),
                field("initialShares", TypeIr::U128),
            ],
        },
        &Value::Record(vec![
            value("subject", Value::Ownership(subject.clone())),
            value("assetA", Value::Bytes(ASSET_A.to_vec())),
            value("assetB", Value::Bytes(ASSET_B.to_vec())),
            value("amountA", Value::Unsigned(AMOUNT_A)),
            value("amountB", Value::Unsigned(AMOUNT_B)),
            value("initialShares", Value::Unsigned(INITIAL_SHARES)),
        ]),
    )?;
    let action = signed_action_for(
        LOCUS_SERVICE_KEY,
        "createPool",
        controller_signing_key,
        payload,
    )?;
    let state = FullState::from_pairs([
        (grant_key.as_slice(), &[1u8][..]),
        (asset_a_key.as_slice(), asset_value.as_slice()),
        (asset_b_key.as_slice(), asset_value.as_slice()),
        (balance_a_key.as_slice(), balance_a.as_slice()),
        (balance_b_key.as_slice(), balance_b.as_slice()),
    ])
    .map_err(|error| anyhow::anyhow!("building createPool state: {error:?}"))?;

    let keys = plan_action(engine, artifact, &action, state.clone())
        .context("Test C: Matrix Locus createPool planner")?;
    println!("MATRIX_CREATE_POOL_PLAN=PASS");
    let output = refine_action(engine, artifact, &action, state.clone(), &keys)
        .context("Test C: Matrix Locus createPool refine")?;
    require_applied(&output, "Matrix Locus createPool")?;
    println!("MATRIX_CREATE_POOL_RECEIPT=APPLIED");
    let next_state = apply_recovery(&state, &output, "Matrix Locus createPool")?;
    if read_u128(&next_state, &balance_a_key)? != 900
        || read_u128(&next_state, &balance_b_key)? != 1_600
    {
        bail!("Matrix createPool did not debit the initial asset balances");
    }
    let expected_pool = encode_locus_pool_value(AMOUNT_A, AMOUNT_B, INITIAL_SHARES)?;
    if read_required(&next_state, &pool_key)? != expected_pool {
        bail!("Matrix createPool did not write the expected pool reserves and shares");
    }
    println!("MATRIX_CREATE_POOL_STATE=PASS");

    let mut share_key_material = Vec::with_capacity(96);
    share_key_material.extend_from_slice(&ASSET_A);
    share_key_material.extend_from_slice(&ASSET_B);
    share_key_material.extend_from_slice(&subject_key);
    let share_key = application_key_v1(b"locus.liquidity-share.v1", &share_key_material)
        .map_err(|error| anyhow::anyhow!("building liquidity shares key: {error:?}"))?;
    if read_required(&next_state, &share_key)? != INITIAL_SHARES.to_le_bytes() {
        bail!("Matrix createPool did not mint the expected LP position");
    }
    println!("MATRIX_CREATE_POOL_LP_POSITION=PASS");
    if next_state.root() != output.new_root {
        bail!("Matrix createPool new root does not match canonical state");
    }
    println!("MATRIX_CREATE_POOL=PASS");
    Ok(())
}

fn run_locus_swap(
    engine: &Engine,
    artifact: &[u8],
    subject: &Ownership,
    controller: &Ownership,
    controller_signing_key: &SigningKey,
) -> Result<()> {
    const ASSET_IN: [u8; 32] = [0x51; 32];
    const ASSET_OUT: [u8; 32] = [0x52; 32];
    const RESERVE_IN: u128 = 1_000;
    const RESERVE_OUT: u128 = 5_000;
    const TOTAL_SHARES: u128 = 2_000;
    const AMOUNT_IN: u128 = 10;
    const AMOUNT_OUT: u128 = 44;

    let subject_key = ownership_key(subject)?;
    let grant_key = controller_grant_key_for(LOCUS_GRANT_SCHEMA, subject, controller)?;
    let asset_in_key = application_key_v1(b"locus.asset.v2", &ASSET_IN)
        .map_err(|error| anyhow::anyhow!("building swap asset-in key: {error:?}"))?;
    let asset_out_key = application_key_v1(b"locus.asset.v2", &ASSET_OUT)
        .map_err(|error| anyhow::anyhow!("building swap asset-out key: {error:?}"))?;
    let asset_value = encode_locus_asset_value(subject)?;
    let pool_key = locus_pool_key(&ASSET_IN, &ASSET_OUT)?;
    let pool_value = encode_locus_pool_value(RESERVE_IN, RESERVE_OUT, TOTAL_SHARES)?;
    let balance_in_key = locus_balance_key(&ASSET_IN, &subject_key)?;
    let balance_out_key = locus_balance_key(&ASSET_OUT, &subject_key)?;
    let balance_in = 100u128.to_le_bytes();
    let balance_out = 0u128.to_le_bytes();
    let payload = encode_value(
        &TypeIr::Record {
            fields: vec![
                field("subject", TypeIr::Ownership),
                field("assetIn", TypeIr::FixedBytes { len: 32 }),
                field("assetOut", TypeIr::FixedBytes { len: 32 }),
                field("amountIn", TypeIr::U128),
                field("minAmountOut", TypeIr::U128),
            ],
        },
        &Value::Record(vec![
            value("subject", Value::Ownership(subject.clone())),
            value("assetIn", Value::Bytes(ASSET_IN.to_vec())),
            value("assetOut", Value::Bytes(ASSET_OUT.to_vec())),
            value("amountIn", Value::Unsigned(AMOUNT_IN)),
            value("minAmountOut", Value::Unsigned(40)),
        ]),
    )?;
    let action = signed_action_for(
        LOCUS_SERVICE_KEY,
        "swapExactIn",
        controller_signing_key,
        payload,
    )?;
    let state = FullState::from_pairs([
        (grant_key.as_slice(), &[1u8][..]),
        (asset_in_key.as_slice(), asset_value.as_slice()),
        (asset_out_key.as_slice(), asset_value.as_slice()),
        (pool_key.as_slice(), pool_value.as_slice()),
        (balance_in_key.as_slice(), balance_in.as_slice()),
        (balance_out_key.as_slice(), balance_out.as_slice()),
    ])
    .map_err(|error| anyhow::anyhow!("building swap state: {error:?}"))?;

    let keys = plan_action(engine, artifact, &action, state.clone())
        .context("Test C: Matrix Locus swap planner")?;
    println!("MATRIX_SWAP_PLAN=PASS");
    let output = refine_action(engine, artifact, &action, state.clone(), &keys)
        .context("Test C: Matrix Locus swap refine")?;
    require_applied(&output, "Matrix Locus swap")?;
    println!("MATRIX_SWAP_RECEIPT=APPLIED");
    let next_state = apply_recovery(&state, &output, "Matrix Locus swap")?;
    if read_u128(&next_state, &balance_in_key)? != 90
        || read_u128(&next_state, &balance_out_key)? != AMOUNT_OUT
    {
        bail!("Matrix swap did not update the trader balances");
    }
    let expected_pool = encode_locus_pool_value(
        RESERVE_IN + AMOUNT_IN,
        RESERVE_OUT - AMOUNT_OUT,
        TOTAL_SHARES,
    )?;
    if read_required(&next_state, &pool_key)? != expected_pool {
        bail!("Matrix swap did not update pool reserves");
    }
    if next_state.root() != output.new_root {
        bail!("Matrix swap new root does not match canonical state");
    }
    println!("MATRIX_SWAP_RESERVES=PASS");
    println!("MATRIX_SWAP_BALANCES=PASS");
    println!("MATRIX_SWAP=PASS");
    Ok(())
}

fn field(name: &str, ty: TypeIr) -> FieldIr {
    FieldIr {
        name: name.into(),
        ty,
    }
}

fn value(name: &str, value: Value) -> (String, Value) {
    (name.into(), value)
}

fn ownership_key(owner: &Ownership) -> Result<[u8; 32]> {
    owner
        .key()
        .map(|key| key.into())
        .map_err(|error| anyhow::anyhow!("deriving ownership key: {error:?}"))
}

fn encode_locus_asset_value(issuer: &Ownership) -> Result<Vec<u8>> {
    encode_value(
        &TypeIr::Record {
            fields: vec![
                field("version", TypeIr::U8),
                field("issuer", TypeIr::Ownership),
                field("name", TypeIr::Bytes { max: 64 }),
                field("symbol", TypeIr::Bytes { max: 16 }),
                field("decimals", TypeIr::U8),
                field("totalSupply", TypeIr::U128),
            ],
        },
        &Value::Record(vec![
            value("version", Value::Unsigned(2)),
            value("issuer", Value::Ownership(issuer.clone())),
            value("name", Value::Bytes(b"Matrix probe asset".to_vec())),
            value("symbol", Value::Bytes(b"MPRB".to_vec())),
            value("decimals", Value::Unsigned(0)),
            value("totalSupply", Value::Unsigned(2_000)),
        ]),
    )
    .map_err(Into::into)
}

fn locus_pool_key(asset0: &[u8; 32], asset1: &[u8; 32]) -> Result<Vec<u8>> {
    let mut canonical = Vec::with_capacity(64);
    canonical.extend_from_slice(asset0);
    canonical.extend_from_slice(asset1);
    application_key_v1(b"locus.pool.v2", &canonical)
        .map_err(|error| anyhow::anyhow!("building Locus pool key: {error:?}"))
}

fn encode_locus_pool_value(reserve0: u128, reserve1: u128, shares: u128) -> Result<Vec<u8>> {
    encode_value(
        &TypeIr::Record {
            fields: vec![
                field("version", TypeIr::U8),
                field("reserve0", TypeIr::U128),
                field("reserve1", TypeIr::U128),
                field("totalShares", TypeIr::U128),
            ],
        },
        &Value::Record(vec![
            value("version", Value::Unsigned(2)),
            value("reserve0", Value::Unsigned(reserve0)),
            value("reserve1", Value::Unsigned(reserve1)),
            value("totalShares", Value::Unsigned(shares)),
        ]),
    )
    .map_err(Into::into)
}

fn apply_recovery(
    state: &FullState,
    output: &RuntimeRefineOutputV1,
    label: &str,
) -> Result<FullState> {
    if output.parent_root != state.root() {
        bail!("{label} parent root does not match fixture state");
    }
    let recovery = StateRecoveryV1::decode(&output.recovery_payload)
        .map_err(|error| anyhow::anyhow!("decoding {label} state recovery: {error:?}"))?;
    state
        .apply_diff(&recovery.diff)
        .map_err(|error| anyhow::anyhow!("applying {label} state diff: {error:?}"))
}

fn read_required(state: &FullState, key: &[u8]) -> Result<Vec<u8>> {
    state
        .get(key)
        .map_err(|error| anyhow::anyhow!("reading canonical state: {error:?}"))?
        .context("canonical state value is missing")
}

fn read_u128(state: &FullState, key: &[u8]) -> Result<u128> {
    let bytes = state
        .get(key)
        .map_err(|error| anyhow::anyhow!("reading canonical balance: {error:?}"))?
        .context("canonical balance is missing")?;
    let value: [u8; 16] = bytes
        .as_slice()
        .try_into()
        .context("canonical balance has invalid u128 encoding")?;
    Ok(u128::from_le_bytes(value))
}

fn plan_action(
    engine: &Engine,
    artifact: &[u8],
    action: &[u8],
    state: FullState,
) -> Result<Vec<Vec<u8>>> {
    let selector = signed_selector(action)?;
    let ownership_kind = signed_controller_kind(action)?;
    // Match the transaction planner's initial access plan: SignedActionV2
    // always starts with its controller nonce key before discovering app keys.
    let decoded = jamscript_runtime_core::decode_signed_action_v2(action)
        .map_err(|error| anyhow::anyhow!("decoding SignedActionV2 for planner: {error:?}"))?;
    let nonce_key = jamscript_runtime_core::ownership_nonce_key(&decoded.controller)
        .map_err(|error| anyhow::anyhow!("building controller nonce key: {error:?}"))?;
    let mut keys = vec![nonce_key];
    for _ in 0..16 {
        let witness = state_witness(&state, &keys)?;
        let input = RuntimeRefineInputV1 {
            version: RuntimeRefineInputV1::VERSION,
            managed_state: witness,
            external_state: Vec::new(),
            actions: vec![action.to_vec()],
        };
        let encoded = input
            .encode()
            .map_err(|error| anyhow::anyhow!("encoding planner input: {error:?}"))?;
        let output = invoke(
            engine,
            artifact,
            "jamscript_plan_v1",
            &encoded,
            &selector,
            ownership_kind,
        )?;
        if output == [0] {
            return Ok(keys);
        }
        match decode_planner_need_state(&output) {
            Ok(key) if !keys.contains(&key) => keys.push(key),
            Ok(key) => bail!(
                "planner repeated a known key: selector={} ownershipKind={} key={}",
                hex(&selector), ownership_kind, hex(&key)
            ),
            Err(error) => bail!(
                "planner returned an unclassified output: selector={} ownershipKind={} output={} error={error:?}",
                hex(&selector), ownership_kind, hex(&output)
            ),
        }
    }
    bail!(
        "planner exceeded state discovery limit: selector={} ownershipKind={}",
        hex(&selector),
        ownership_kind
    )
}

fn refine_action(
    engine: &Engine,
    artifact: &[u8],
    action: &[u8],
    state: FullState,
    keys: &[Vec<u8>],
) -> Result<RuntimeRefineOutputV1> {
    let input = RuntimeRefineInputV1 {
        version: RuntimeRefineInputV1::VERSION,
        managed_state: state_witness(&state, keys)?,
        external_state: Vec::new(),
        actions: vec![action.to_vec()],
    };
    let action = input
        .actions
        .first()
        .context("refine input has no action")?;
    let selector = signed_selector(action)?;
    let ownership_kind = signed_controller_kind(action)?;
    let output = invoke(
        engine,
        artifact,
        "minijam_refine",
        &input
            .encode()
            .map_err(|error| anyhow::anyhow!("encoding refine input: {error:?}"))?,
        &selector,
        ownership_kind,
    )?;
    RuntimeRefineOutputV1::decode(&output)
        .map_err(|error| anyhow::anyhow!("decoding PVM action receipt: {error:?}"))
}

fn state_witness(state: &FullState, keys: &[Vec<u8>]) -> Result<ManagedStateWitnessV1> {
    let refs = keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let proof = state
        .proof_for(&refs)
        .map_err(|error| anyhow::anyhow!("building storage proof: {error:?}"))?;
    Ok(ManagedStateWitnessV1 {
        version: ManagedStateWitnessV1::VERSION,
        parent_root: state.root(),
        access_plan: StateAccessPlanV1::from_keys(keys)
            .map_err(|error| anyhow::anyhow!("building access plan: {error:?}"))?,
        storage_proof: proof.into_nodes().into_iter().collect(),
    })
}

fn invoke(
    engine: &Engine,
    artifact: &[u8],
    export: &str,
    input: &[u8],
    selector: &[u8; 8],
    ownership_kind: &str,
) -> Result<Vec<u8>> {
    let module = Module::new(engine, &ModuleConfig::new(), artifact.to_vec().into())?;
    let mut linker: Linker<(), MemoryAccessError> = Linker::new();
    let input_for_fetch = input.to_vec();
    linker.define_untyped("minijam_fetch", move |caller| {
        let output = caller.instance.reg(Reg::A0) as u32;
        let offset = caller.instance.reg(Reg::A1) as usize;
        let capacity = caller.instance.reg(Reg::A2) as usize;
        let mode = caller.instance.reg(Reg::A3);
        let index = caller.instance.reg(Reg::A4);
        if mode != 13 || index != 0 || offset > input_for_fetch.len() {
            caller.instance.set_reg(Reg::A0, u64::MAX);
            return Ok(());
        }
        let remaining = &input_for_fetch[offset..];
        if remaining.len() <= capacity {
            caller.instance.write_memory(output, remaining)?;
        }
        caller.instance.set_reg(Reg::A0, remaining.len() as u64);
        Ok(())
    })?;
    linker.define_untyped("minijam_network_domain", |caller| {
        let output = caller.instance.reg(Reg::A0) as u32;
        let capacity = caller.instance.reg(Reg::A1) as usize;
        let output_size = caller.instance.reg(Reg::A2) as u32;
        if capacity < NETWORK_DOMAIN.len() {
            caller.instance.set_reg(Reg::A0, u64::MAX);
            return Ok(());
        }
        caller.instance.write_memory(output, &NETWORK_DOMAIN)?;
        caller
            .instance
            .write_memory(output_size, &(NETWORK_DOMAIN.len() as u64).to_le_bytes())?;
        caller.instance.set_reg(Reg::A0, 0);
        Ok(())
    })?;
    linker.define_untyped("minijam_log", |caller| {
        let pointer = caller.instance.reg(Reg::A3) as u32;
        let length = caller.instance.reg(Reg::A4) as usize;
        if length <= 1024 {
            if let Ok(bytes) = caller.instance.read_memory(pointer, length as u32) {
                eprintln!("GUEST_DIAGNOSTIC {}", String::from_utf8_lossy(&bytes));
            }
        }
        caller.instance.set_reg(Reg::A0, 0);
        Ok(())
    })?;
    linker.define_fallback(|caller, _| {
        caller.instance.set_reg(Reg::A0, u64::MAX);
        Ok(())
    });

    let pre = linker.instantiate_pre(&module)?;
    let mut instance = pre.instantiate()?;
    instance.set_gas(PVM_GAS);
    if let Err(error) = instance.call_typed_and_get_result::<u64, _>(&mut (), export, ()) {
        let failure_kind = match &error {
            CallError::Trap => "TRAP",
            CallError::NotEnoughGas => "OUT_OF_GAS",
            CallError::Error(_) => "HOST_ERROR",
            CallError::User(_) => "HOST_ERROR",
            CallError::Step => "STEP",
        };
        let pc = instance.program_counter();
        eprintln!(
            "PVM_INSTRUCTION_WINDOW {}",
            pvm_instruction_window(artifact, pc)
        );
        bail!(
            "PVM_FAILURE serviceId=test-fixture artifactDigest={} export={export} selector={} ownershipKind={} kind={failure_kind} error={error:?} pc={:?} sourceLocation={} gasRemaining={:?}",
            hex(&service_runtime_core::blake2_256(artifact)),
            hex(selector),
            ownership_kind,
            instance.program_counter(),
            pvm_source_location(artifact, instance.program_counter()),
            instance.gas()
        );
    }
    let pointer = instance.reg(Reg::A0) as u32;
    let length = instance.reg(Reg::A1) as u32;
    Ok(instance.read_memory(pointer, length)?)
}

fn pvm_instruction_window(artifact: &[u8], pc: Option<ProgramCounter>) -> String {
    let Some(pc) = pc else {
        return "unavailable".into();
    };
    let Ok(blob) = ProgramBlob::parse(artifact.to_vec().into()) else {
        return "unavailable".into();
    };
    let start = pc.0.saturating_sub(16);
    let end = pc.0.saturating_add(16);
    let mut entries = Vec::new();
    for instruction in blob.instructions() {
        if instruction.offset.0 >= start && instruction.offset.0 <= end {
            entries.push(instruction.to_string());
        }
        if instruction.offset.0 > end {
            break;
        }
    }
    entries.join(" | ")
}

fn pvm_source_location(artifact: &[u8], pc: Option<ProgramCounter>) -> String {
    let Some(pc) = pc else {
        return "unavailable".into();
    };
    let Ok(blob) = ProgramBlob::parse(artifact.to_vec().into()) else {
        return "unavailable".into();
    };
    let Ok(Some(mut line_program)) = blob.get_debug_line_program_at(pc) else {
        return "unavailable".into();
    };
    for _ in 0..128 {
        let Ok(Some(region)) = line_program.run() else {
            break;
        };
        if !region.instruction_range().contains(&pc) {
            continue;
        }
        for frame in region.frames() {
            if let Ok(Some(location)) = frame.location() {
                return format!(
                    "{}:{}",
                    frame
                        .full_name()
                        .map(|name| name.to_string())
                        .unwrap_or_default(),
                    location
                );
            }
        }
        break;
    }
    "unavailable".into()
}

fn require_applied(output: &RuntimeRefineOutputV1, label: &str) -> Result<()> {
    let receipt = output
        .receipts
        .first()
        .context("PVM output has no receipt")?;
    if receipt.status != ActionStatusV1::Applied || receipt.error_code.is_some() {
        bail!(
            "{label}: expected APPLIED, got {:?}/{:?}",
            receipt.status,
            receipt.error_code
        );
    }
    Ok(())
}

fn require_abort(output: &RuntimeRefineOutputV1, expected: u32, label: &str) -> Result<()> {
    let receipt = output
        .receipts
        .first()
        .context("PVM output has no receipt")?;
    if receipt.status != ActionStatusV1::Failed || receipt.error_code != Some(expected) {
        bail!(
            "{label}: expected structured abort {expected}, got {:?}/{:?}",
            receipt.status,
            receipt.error_code
        );
    }
    Ok(())
}

fn signed_selector(action: &[u8]) -> Result<[u8; 8]> {
    let decoded = jamscript_runtime_core::decode_signed_action_v2(action)
        .map_err(|error| anyhow::anyhow!("decoding SignedActionV2: {error:?}"))?;
    Ok(decoded.action_selector)
}

fn signed_controller_kind(action: &[u8]) -> Result<&'static str> {
    let decoded = jamscript_runtime_core::decode_signed_action_v2(action)
        .map_err(|error| anyhow::anyhow!("decoding SignedActionV2: {error:?}"))?;
    Ok(match decoded.controller.kind {
        OwnershipKind::Ed25519Key => "ED25519",
        OwnershipKind::Sr25519Key => "SR25519",
        OwnershipKind::Secp256k1Key => "SECP256K1",
        OwnershipKind::Secp256k1Keccak20 => "EVM",
        OwnershipKind::MulticryptoAccount32 => "MULTICRYPTO",
    })
}

fn make_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.set_backend(Some(BackendKind::Interpreter));
    Ok(Engine::new(&config)?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
