use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use ownership_matrix::MatrixControlClaimProofV1;
use polkavm::{BackendKind, Config, Engine, Linker, MemoryAccessError, Module, ModuleConfig, Reg};
use service_runtime_core::{
    ManagedStateWitnessV1, RuntimeRefineInputV1, RuntimeRefineOutputV1, StateAccessPlanV1,
    EMPTY_STATE_ROOT_V1,
};
use std::{env, fs, path::PathBuf};

const PVM_GAS: i64 = 80_000_000;
const PROOF_CAPACITY: usize = 2048;

fn main() -> Result<()> {
    let artifact = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: matrix_adapter <service.pvm>")?;
    let artifact = fs::read(&artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let master = SigningKey::from_bytes(&[1; 32]);
    let self_signing = SigningKey::from_bytes(&[2; 32]);
    let device = SigningKey::from_bytes(&[3; 32]);
    let master_public = master.verifying_key().to_bytes();
    let self_signing_public = self_signing.verifying_key().to_bytes();
    let device_public = device.verifying_key().to_bytes();

    let mut proof = MatrixControlClaimProofV1 {
        user_id: "@alice:example.org".into(),
        self_signing_public_key: self_signing_public,
        master_signature: [0; 64],
        device_id: "LOCUS_TEST".into(),
        algorithms: vec!["m.olm.v1.curve25519-aes-sha2".into()],
        device_curve25519_key: [4; 32],
        device_ed25519_key: device_public,
        self_signing_signature: [0; 64],
    };
    proof.master_signature = master
        .sign(&proof.canonical_self_signing_object()?)
        .to_bytes();
    proof.self_signing_signature = self_signing
        .sign(&proof.canonical_device_keys_object()?)
        .to_bytes();
    let encoded = proof.encode()?;
    if !proof.verify_for(&master_public, &device_public).is_ok() {
        bail!("generated M→S→D proof did not verify before PVM execution");
    }

    let engine = make_engine()?;
    expect_code(
        run_probe(
            &engine,
            &artifact,
            &master_public,
            &device_public,
            &encoded,
            encoded.len() as u16,
        )?,
        5098,
        "valid M→S→D proof",
    )?;
    println!("MATRIX_ADAPTER_PVM_VALID=PASS");

    let mut invalid_master = proof.clone();
    invalid_master.master_signature[0] ^= 1;
    let invalid_master = invalid_master.encode()?;
    expect_code(
        run_probe(
            &engine,
            &artifact,
            &master_public,
            &device_public,
            &invalid_master,
            invalid_master.len() as u16,
        )?,
        5005,
        "invalid M→S signature",
    )?;
    println!("MATRIX_ADAPTER_PVM_INVALID_M_TO_S=PASS");

    let mut invalid_device = proof.clone();
    invalid_device.self_signing_signature[0] ^= 1;
    let invalid_device = invalid_device.encode()?;
    expect_code(
        run_probe(
            &engine,
            &artifact,
            &master_public,
            &device_public,
            &invalid_device,
            invalid_device.len() as u16,
        )?,
        5005,
        "invalid S→D signature",
    )?;
    println!("MATRIX_ADAPTER_PVM_INVALID_S_TO_D=PASS");

    expect_code(
        run_probe(
            &engine,
            &artifact,
            &[8; 32],
            &device_public,
            &encoded,
            encoded.len() as u16,
        )?,
        5005,
        "wrong subject",
    )?;
    println!("MATRIX_ADAPTER_PVM_WRONG_SUBJECT=PASS");

    expect_code(
        run_probe(
            &engine,
            &artifact,
            &master_public,
            &[9; 32],
            &encoded,
            encoded.len() as u16,
        )?,
        5005,
        "wrong controller",
    )?;
    println!("MATRIX_ADAPTER_PVM_WRONG_CONTROLLER=PASS");

    expect_code(
        run_probe(&engine, &artifact, &master_public, &device_public, &[1], 1)?,
        5005,
        "truncated proof",
    )?;
    expect_code(
        run_probe(
            &engine,
            &artifact,
            &master_public,
            &device_public,
            &encoded,
            (PROOF_CAPACITY + 1) as u16,
        )?,
        5005,
        "oversized proof length",
    )?;
    println!("MATRIX_ADAPTER_PVM_MALFORMED=PASS");
    println!("MATRIX_ADAPTER_PVM=PASS");
    println!("MATRIX_ADAPTER_PVM_FATAL=false");
    Ok(())
}

fn expect_code(actual: u32, expected: u32, label: &str) -> Result<()> {
    if actual != expected {
        bail!("{label} returned {actual:#010x}; expected {expected:#010x}");
    }
    Ok(())
}

fn make_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.set_backend(Some(BackendKind::Interpreter));
    Ok(Engine::new(&config)?)
}

fn run_probe(
    engine: &Engine,
    artifact: &[u8],
    subject: &[u8; 32],
    controller: &[u8; 32],
    proof: &[u8],
    proof_length: u16,
) -> Result<u32> {
    let payload = encode_probe_action(subject, controller, proof, proof_length)?;
    let refine_input = RuntimeRefineInputV1 {
        version: RuntimeRefineInputV1::VERSION,
        managed_state: ManagedStateWitnessV1 {
            version: ManagedStateWitnessV1::VERSION,
            parent_root: EMPTY_STATE_ROOT_V1,
            access_plan: StateAccessPlanV1::from_keys(core::iter::empty::<&[u8]>())
                .map_err(|error| anyhow::anyhow!("building empty access plan: {error:?}"))?,
            storage_proof: Vec::new(),
        },
        external_state: Vec::new(),
        actions: vec![payload],
    }
    .encode()
    .map_err(|error| anyhow::anyhow!("encoding PVM refine input: {error:?}"))?;

    let module = Module::new(engine, &ModuleConfig::new(), artifact.to_vec().into())?;
    let mut linker: Linker<(), MemoryAccessError> = Linker::new();
    let refine_input_for_fetch = refine_input.clone();
    linker.define_untyped("minijam_fetch", move |caller| {
        let output = caller.instance.reg(Reg::A0) as u32;
        let offset = caller.instance.reg(Reg::A1) as usize;
        let capacity = caller.instance.reg(Reg::A2) as usize;
        let mode = caller.instance.reg(Reg::A3);
        let index = caller.instance.reg(Reg::A4);
        if mode != 13 || index != 0 || offset > refine_input_for_fetch.len() {
            caller.instance.set_reg(Reg::A0, u64::MAX);
            return Ok(());
        }
        let remaining = &refine_input_for_fetch[offset..];
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
        if capacity < 32 {
            caller.instance.set_reg(Reg::A0, u64::MAX);
            return Ok(());
        }
        caller.instance.write_memory(output, &[0u8; 32])?;
        caller.instance.write_memory(output_size, &(32u64).to_le_bytes())?;
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
    instance
        .call_typed_and_get_result::<u64, _>(&mut (), "minijam_refine", ())
        .map_err(|error| anyhow::anyhow!("real PVM refine call: {error:?}"))?;
    let output_ptr = instance.reg(Reg::A0) as u32;
    let output_len = instance.reg(Reg::A1) as u32;
    let output = instance.read_memory(output_ptr, output_len)?;
    let decoded = RuntimeRefineOutputV1::decode(&output).map_err(|error| {
        anyhow::anyhow!("decoding PVM receipt (fatal output is not an application rejection): {error:?}")
    })?;
    let receipt = decoded
        .receipts
        .first()
        .context("PVM refine output has no action receipt")?;
    receipt
        .error_code
        .context("probe action returned without the expected ordinary abort")
}

fn encode_probe_action(
    subject: &[u8; 32],
    controller: &[u8; 32],
    proof: &[u8],
    proof_length: u16,
) -> Result<Vec<u8>> {
    if proof.len() > PROOF_CAPACITY {
        bail!("test proof exceeds fixed action input capacity");
    }
    let mut action = Vec::with_capacity(2 + 34 + 34 + 2 + PROOF_CAPACITY);
    action.extend_from_slice(&[1, 0]);
    action.extend_from_slice(subject);
    action.extend_from_slice(&[1, 0]);
    action.extend_from_slice(controller);
    action.extend_from_slice(&proof_length.to_le_bytes());
    action.extend_from_slice(proof);
    action.resize(action.len() + PROOF_CAPACITY - proof.len(), 0);
    Ok(action)
}
