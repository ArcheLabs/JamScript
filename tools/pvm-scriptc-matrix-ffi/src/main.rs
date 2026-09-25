use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use jamscript_crypto::{verify_matrix_cross_signing, MatrixControlClaimProofV1};
use polkavm::{BackendKind, Config, Engine, Linker, MemoryAccessError, Module, ModuleConfig, Reg};
use service_runtime_core::{
    ManagedStateWitnessV1, RuntimeRefineInputV1, RuntimeRefineOutputV1, StateAccessPlanV1,
    EMPTY_STATE_ROOT_V1,
};
use std::{env, fs, path::PathBuf};

const EXPECTED_PROOF_LEN: usize = 334;
const PVM_GAS: i64 = 80_000_000;

fn main() -> Result<()> {
    let artifact = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: pvm-scriptc-matrix-ffi <service.pvm>")?;
    let artifact = fs::read(&artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let (subject, controller, proof) = valid_proof();
    if proof.len() != EXPECTED_PROOF_LEN {
        bail!("test proof length is {}, expected {EXPECTED_PROOF_LEN}", proof.len());
    }
    if !verify_matrix_cross_signing(&subject, &controller, &proof) {
        bail!("test proof did not pass the direct Rust verifier");
    }
    println!("MATRIX_NATIVE_RUST_DIRECT=PASS");

    let mut invalid_proof = proof.clone();
    *invalid_proof.last_mut().context("empty test proof")? ^= 1;
    let c_abi_valid = unsafe {
        service_runtime_guest::jamscript_verify_matrix_cross_signing(
            subject.as_ptr(), subject.len(),
            controller.as_ptr(), controller.len(),
            proof.as_ptr(), proof.len(),
        )
    };
    let c_abi_invalid = unsafe {
        service_runtime_guest::jamscript_verify_matrix_cross_signing(
            subject.as_ptr(), subject.len(),
            controller.as_ptr(), controller.len(),
            invalid_proof.as_ptr(), invalid_proof.len(),
        )
    };
    if c_abi_valid != 1 || c_abi_invalid != 0 {
        bail!("C ABI verifier returned valid={c_abi_valid}, invalid={c_abi_invalid}; expected 1/0");
    }
    println!("MATRIX_NATIVE_C_ABI=PASS");

    let engine = make_engine()?;
    let plain_code = run_probe(0, &engine, &artifact, &subject, &controller, &proof)?;
    if plain_code != 5098 {
        bail!("plain action returned {plain_code:#010x}; expected ordinary abort 5098");
    }
    println!("MATRIX_SCRIPTC_PVM_PLAIN_ACTION=PASS (5098)");

    let invalid_code = run_probe(1, &engine, &artifact, &subject, &controller, &invalid_proof)?;
    if invalid_code != 5005 {
        bail!("tampered proof returned {invalid_code:#010x}; expected ordinary abort 5005");
    }
    println!("MATRIX_SCRIPTC_PVM_INVALID_PROOF=PASS (5005)");

    let valid_code = run_probe(1, &engine, &artifact, &subject, &controller, &proof)?;
    if valid_code != 5098 {
        bail!("valid proof returned {valid_code:#010x}; expected ordinary probe abort 5098");
    }
    println!("MATRIX_SCRIPTC_PVM_VALID_PROOF=PASS (5098)");

    println!("MATRIX_SCRIPTC_PVM_NO_FATAL_UNCAUGHT=PASS");
    Ok(())
}

fn make_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.set_backend(Some(BackendKind::Interpreter));
    Ok(Engine::new(&config)?)
}

fn run_probe(
    stage: u8,
    engine: &Engine,
    artifact: &[u8],
    subject: &[u8; 32],
    controller: &[u8; 32],
    proof: &[u8],
) -> Result<u32> {
    let payload = encode_probe_action(stage, subject, controller, proof);
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
        caller
            .instance
            .write_memory(output_size, &(32u64).to_le_bytes())?;
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
        anyhow::anyhow!(
            "decoding PVM refine receipt (a fatal output is not an application rejection): {error:?}; ptr={output_ptr:#x}, len={output_len}, bytes={output:?}"
        )
    })?;
    let receipt = decoded
        .receipts
        .first()
        .context("PVM refine output has no action receipt")?;
    receipt
        .error_code
        .context("probe action unexpectedly applied without its expected abort")
}

fn encode_probe_action(stage: u8, subject: &[u8; 32], controller: &[u8; 32], proof: &[u8]) -> Vec<u8> {
    let mut action = Vec::with_capacity(1 + subject.len() + controller.len() + proof.len());
    action.push(stage);
    action.extend_from_slice(subject);
    action.extend_from_slice(controller);
    // fixedBytes<334> has no length prefix in the JamScript ABI.
    action.extend_from_slice(proof);
    action
}

fn valid_proof() -> ([u8; 32], [u8; 32], Vec<u8>) {
    let master = SigningKey::from_bytes(&[11; 32]);
    let self_signing = SigningKey::from_bytes(&[12; 32]);
    let device = SigningKey::from_bytes(&[13; 32]);
    let mut proof = MatrixControlClaimProofV1 {
        user_id: "@a:x".into(),
        self_signing_public_key: self_signing.verifying_key().to_bytes(),
        master_signature: [0; 64],
        device_id: "DEV".into(),
        algorithms: vec!["a".repeat(95)],
        device_curve25519_key: [14; 32],
        device_ed25519_key: device.verifying_key().to_bytes(),
        self_signing_signature: [0; 64],
    };
    proof.master_signature = master
        .sign(&proof.canonical_self_signing_object().expect("canonical master object"))
        .to_bytes();
    proof.self_signing_signature = self_signing
        .sign(&proof.canonical_device_keys_object().expect("canonical device object"))
        .to_bytes();
    (
        master.verifying_key().to_bytes(),
        device.verifying_key().to_bytes(),
        proof.encode().expect("encode test Matrix proof"),
    )
}
