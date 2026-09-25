use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use jamscript_crypto::{blake2_256, verify_ed25519};
use polkavm::{BackendKind, Config, Engine, Linker, MemoryAccessError, Module, ModuleConfig, Reg};
use service_runtime_core::{
    ManagedStateWitnessV1, RuntimeRefineInputV1, RuntimeRefineOutputV1, StateAccessPlanV1,
    EMPTY_STATE_ROOT_V1,
};
use std::{env, fs, path::PathBuf};

const PVM_GAS: i64 = 80_000_000;

fn main() -> Result<()> {
    let artifact = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: pvm-scriptc-ed25519-ffi <service.pvm>")?;
    let artifact = fs::read(&artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let signing_key = SigningKey::from_bytes(&[11; 32]);
    let public_key = signing_key.verifying_key().to_bytes();
    let subject = [33; 32];
    let mut subject_preimage = b"OWNERSHIP_ABSTRACTION_KEY_V1".to_vec();
    subject_preimage.extend_from_slice(&[1, 0, 32, 0]);
    subject_preimage.extend_from_slice(&subject);
    let message = blake2_256(&subject_preimage);
    let signature = signing_key.sign(&message).to_bytes();
    if verify_ed25519(&public_key, &signature, &message).is_err() {
        bail!("generic Ed25519 implementation rejected its valid signature");
    }
    println!("GENERIC_ED25519_RUST=PASS");

    let mut invalid_signature = signature;
    invalid_signature[0] ^= 1;
    let c_abi_valid = unsafe {
        service_runtime_guest::jamscript_verify_ed25519(
            public_key.as_ptr(), public_key.len(),
            message.as_ptr(), message.len(),
            signature.as_ptr(), signature.len(),
        )
    };
    let c_abi_invalid = unsafe {
        service_runtime_guest::jamscript_verify_ed25519(
            public_key.as_ptr(), public_key.len(),
            message.as_ptr(), message.len(),
            invalid_signature.as_ptr(), invalid_signature.len(),
        )
    };
    let c_abi_bad_lengths = unsafe {
        service_runtime_guest::jamscript_verify_ed25519(
            public_key.as_ptr(), public_key.len() - 1,
            message.as_ptr(), message.len(),
            signature.as_ptr(), signature.len(),
        )
    };
    if c_abi_valid != 1 || c_abi_invalid != 0 || c_abi_bad_lengths != 0 {
        bail!("C ABI verifier returned valid={c_abi_valid}, invalid={c_abi_invalid}, bad_lengths={c_abi_bad_lengths}; expected 1/0/0");
    }
    println!("GENERIC_ED25519_C_ABI=PASS");

    let engine = make_engine()?;
    let plain_code = run_probe(0, &engine, &artifact, &public_key, &signature)?;
    if plain_code != 5098 {
        bail!("plain action returned {plain_code:#010x}; expected ordinary abort 5098");
    }
    let invalid_code = run_probe(1, &engine, &artifact, &public_key, &invalid_signature)?;
    if invalid_code != 5005 {
        bail!("invalid signature returned {invalid_code:#010x}; expected ordinary abort 5005");
    }
    let valid_code = run_probe(1, &engine, &artifact, &public_key, &signature)?;
    if valid_code != 5098 {
        bail!("valid signature returned {valid_code:#010x}; expected ordinary probe abort 5098");
    }
    println!("GENERIC_ED25519_PVM=PASS");
    println!("GENERIC_ED25519_FATAL=false");
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
    public_key: &[u8; 32],
    signature: &[u8; 64],
) -> Result<u32> {
    let payload = encode_probe_action(stage, public_key, signature);
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
        anyhow::anyhow!("decoding PVM receipt (a fatal output is not an application rejection): {error:?}; ptr={output_ptr:#x}, len={output_len}, bytes={output:?}")
    })?;
    let receipt = decoded.receipts.first().context("PVM refine output has no action receipt")?;
    receipt.error_code.context("probe action unexpectedly applied without its expected abort")
}

fn encode_probe_action(stage: u8, public_key: &[u8; 32], signature: &[u8; 64]) -> Vec<u8> {
    let mut action = Vec::with_capacity(1 + 36 + public_key.len() + signature.len());
    action.push(stage);
    action.extend_from_slice(&[1, 0, 32, 0]);
    action.extend_from_slice(&[33; 32]);
    action.extend_from_slice(public_key);
    action.extend_from_slice(signature);
    action
}
