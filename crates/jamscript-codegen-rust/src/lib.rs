use jamscript_ir::{action_selector, ActionIr, AuthKind, ComputeIr, ServiceIr};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MiniJamContext {
    pub service_id: u32,
    pub genesis_hash: [u8; 32],
}

pub fn generate_no_std_rust(ir: &ServiceIr) -> Result<String, String> {
    generate_no_std_rust_with_context(ir, MiniJamContext::default())
}

pub fn generate_no_std_rust_with_context(
    ir: &ServiceIr,
    context: MiniJamContext,
) -> Result<String, String> {
    let action = ir
        .actions
        .first()
        .ok_or_else(|| "IR contains no action".to_string())?;
    let selector = action_selector(&action.name);
    let setup = compute_setup(action, "input")?;
    let auth_setup = auth_setup(action, &setup)?;
    Ok(format!(
        r##"#![no_std]
#![allow(static_mut_refs)]

#[repr(C)]
pub struct RefineOutput {{ pub data: *const u8, pub size: usize }}

extern "C" {{
    fn minijam_payload(output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    fn minijam_result_count() -> usize;
    fn minijam_result(index: usize, output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    fn minijam_storage_read(key: *const u8, key_size: usize, output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
    fn minijam_storage_write(key: *const u8, key_size: usize, value: *const u8, value_size: usize) -> u32;
}}

static mut INPUT: [u8; 1048576] = [0; 1048576];
static mut RESULT: [u8; 1048704] = [0; 1048704];
static mut OUTPUT: [u8; 1048704] = [0; 1048704];

const SERVICE_ID: u32 = {service_id};
const GENESIS_HASH: [u8; 32] = {genesis_hash};
const ACTION_SELECTOR: [u8; 8] = {selector};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {{ loop {{}} }}

#[no_mangle]
pub unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, length: usize) -> *mut u8 {{
    for index in 0..length {{ destination.add(index).write(source.add(index).read()); }}
    destination
}}

#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, length: usize) -> *mut u8 {{
    for index in 0..length {{ destination.add(index).write(value as u8); }}
    destination
}}

#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, length: usize) -> i32 {{
    for index in 0..length {{
        let left = left.add(index).read();
        let right = right.add(index).read();
        if left != right {{ return if left < right {{ -1 }} else {{ 1 }}; }}
    }}
    0
}}

fn read_u64(input: &[u8], offset: usize) -> Result<u64, ()> {{
    let bytes = input.get(offset..offset + 8).ok_or(())?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| ())?))
}}

#[no_mangle]
pub extern "C" fn minijam_refine() -> RefineOutput {{
    let mut input_size = 0usize;
    let status = unsafe {{ minijam_payload(INPUT.as_mut_ptr(), 1048576, &mut input_size) }};
    if status != 0 {{ return error_output(1); }}
    let input = unsafe {{ core::slice::from_raw_parts(INPUT.as_ptr(), input_size) }};
    match refine_payload(input) {{
        Ok(size) => RefineOutput {{ data: unsafe {{ OUTPUT.as_ptr() }}, size }},
        Err(code) => error_output(code),
    }}
}}

#[no_mangle]
pub extern "C" fn minijam_accumulate() {{
    let count = unsafe {{ minijam_result_count() }};
    for index in 0..count {{
        let mut size = 0usize;
        let status = unsafe {{ minijam_result(index, RESULT.as_mut_ptr(), 1048704, &mut size) }};
        if status != 0 {{ continue; }}
        let refined = unsafe {{ core::slice::from_raw_parts(RESULT.as_ptr(), size) }};
        let Ok(action) = jamscript_runtime_core::decode_refined_action(refined) else {{ continue; }};
        let key = jamscript_runtime_core::state_key(SERVICE_ID, jamscript_runtime_core::NONCE_SCHEMA_V1, &action.sender);
        let mut nonce_bytes = [0u8; 8];
        let mut nonce_size = 0usize;
        let read_status = unsafe {{ minijam_storage_read(key.as_ptr(), key.len(), nonce_bytes.as_mut_ptr(), nonce_bytes.len(), &mut nonce_size) }};
        let expected = if read_status == 0 && nonce_size == 8 {{ u64::from_le_bytes(nonce_bytes) }} else {{ 0 }};
        if action.nonce != expected {{ continue; }}
        let Some(next) = expected.checked_add(1) else {{ continue; }};
        let next_bytes = next.to_le_bytes();
        let _ = unsafe {{ minijam_storage_write(key.as_ptr(), key.len(), next_bytes.as_ptr(), next_bytes.len()) }};
    }}
}}

fn refine_payload(input: &[u8]) -> Result<usize, u32> {{
    {auth_setup}
}}

fn error_output(code: u32) -> RefineOutput {{
    unsafe {{
        OUTPUT[..4].copy_from_slice(&code.to_le_bytes());
        RefineOutput {{ data: OUTPUT.as_ptr(), size: 4 }}
    }}
}}
"##,
        service_id = context.service_id,
        genesis_hash = byte_array_literal(&context.genesis_hash),
        selector = byte_array_literal(&selector),
        auth_setup = auth_setup,
    ))
}

fn field_offset(action: &ActionIr, field: &str) -> Result<usize, String> {
    action
        .input
        .iter()
        .position(|item| item.name == field)
        .map(|index| index * 8)
        .ok_or_else(|| format!("compute references unknown input field `{field}`"))
}

fn compute_setup(action: &ActionIr, input_name: &str) -> Result<String, String> {
    match &action.compute {
        ComputeIr::ReturnInputField { field } => Ok(format!(
            "let value = read_u64(&{input_name}, {})?;",
            field_offset(action, field)?
        )),
        ComputeIr::AddInputField { field, value } => Ok(format!(
            "let value = read_u64(&{input_name}, {})?.checked_add({value}u128 as u64).ok_or(())?;",
            field_offset(action, field)?
        )),
        ComputeIr::ReturnInteger { value } => Ok(format!("let value: u64 = {value}u128 as u64;")),
        ComputeIr::Unsupported(message) => Err(message.clone()),
    }
}

fn auth_setup(action: &ActionIr, setup: &str) -> Result<String, String> {
    match action.auth {
        AuthKind::Public => Ok(format!(
            "let computed: Result<u64, ()> = (|| {{ {setup} Ok(value) }})();\n    let value = computed.map_err(|_| 1u32)?;\n    let bytes = value.to_le_bytes();\n    unsafe {{ OUTPUT[..8].copy_from_slice(&bytes); }}\n    Ok(8)"
        )),
        AuthKind::Wallet => Ok(format!(
            "let signed = jamscript_runtime_core::decode_signed_action(input).map_err(|error| error as u32)?;\n    let verified = jamscript_runtime_core::verify_signed_action(signed, GENESIS_HASH, SERVICE_ID, ACTION_SELECTOR, None).map_err(|error| error as u32)?;\n    let input = verified.payload;\n    let computed: Result<u64, ()> = (|| {{ {setup} Ok(value) }})();\n    let value = computed.map_err(|_| 1u32)?;\n    let bytes = value.to_le_bytes();\n    let size = jamscript_runtime_core::encode_refined_action(&verified, &bytes, unsafe {{ &mut OUTPUT }}).map_err(|error| error as u32)?;\n    Ok(size)"
        )),
    }
}

fn byte_array_literal<const N: usize>(bytes: &[u8; N]) -> String {
    let values = bytes
        .iter()
        .map(|byte| byte.to_string())
        .collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_ir::{ComputeIr, FieldIr, TypeIr};

    #[test]
    fn emits_public_no_std_exports() {
        let source = generate_no_std_rust(&ServiceIr {
            package_name: "x".into(),
            package_version: "0.1.0".into(),
            actions: vec![ActionIr {
                name: "add".into(),
                auth: AuthKind::Public,
                input: vec![FieldIr {
                    name: "value".into(),
                    ty: TypeIr::U64,
                }],
                compute: ComputeIr::AddInputField {
                    field: "value".into(),
                    value: 1,
                },
            }],
        })
        .unwrap();
        assert!(source.contains("#![no_std]"));
        assert!(source.contains("minijam_refine"));
        assert!(source.contains("checked_add(1u128 as u64)"));
    }

    #[test]
    fn emits_wallet_runtime_verification_and_storage() {
        let source = generate_no_std_rust(&ServiceIr {
            package_name: "x".into(),
            package_version: "0.1.0".into(),
            actions: vec![ActionIr {
                name: "add".into(),
                auth: AuthKind::Wallet,
                input: vec![FieldIr {
                    name: "value".into(),
                    ty: TypeIr::U64,
                }],
                compute: ComputeIr::ReturnInputField {
                    field: "value".into(),
                },
            }],
        })
        .unwrap();
        assert!(source.contains("verify_signed_action"));
        assert!(source.contains("minijam_storage_write"));
        assert!(source.contains(
            "verify_signed_action(signed, GENESIS_HASH, SERVICE_ID, ACTION_SELECTOR, None)"
        ));
    }
}
