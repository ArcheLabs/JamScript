use jamscript_ir::{action_selector, ActionIr, AuthKind, CommitIr, ComputeIr, ServiceIr, TypeIr};

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
    let decoder = payload_decoder(action)?;
    let setup = compute_setup(action, "decoded", &ir.native_imports)?;
    let auth_setup = auth_setup(action, &setup);
    let native_declarations = native_declarations(ir);
    let commit = accumulate_commit(action, ir)?;
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
{native_declarations}}}

static mut INPUT: [u8; 1048576] = [0; 1048576];
static mut RESULT: [u8; 1048704] = [0; 1048704];
static mut OUTPUT: [u8; 1048704] = [0; 1048704];

const SERVICE_ID: u32 = {service_id};
const GENESIS_HASH: [u8; 32] = {genesis_hash};
const ACTION_SELECTOR: [u8; 8] = {selector};
const NATIVE_ERROR_BASE: u32 = 0x8000_0000;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {{ loop {{}} }}

#[no_mangle]
pub unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, length: usize) -> *mut u8 {{
    for index in 0..length {{ destination.add(index).write(source.add(index).read()); }} destination
}}
#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, length: usize) -> *mut u8 {{
    for index in 0..length {{ destination.add(index).write(value as u8); }} destination
}}
#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, length: usize) -> i32 {{
    for index in 0..length {{ let a = left.add(index).read(); let b = right.add(index).read(); if a != b {{ return if a < b {{ -1 }} else {{ 1 }}; }} }} 0
}}

struct PayloadReader<'a> {{ input: &'a [u8], offset: usize }}
impl<'a> PayloadReader<'a> {{
    fn take(&mut self, length: usize) -> Result<&'a [u8], ()> {{
        let end = self.offset.checked_add(length).ok_or(())?;
        let value = self.input.get(self.offset..end).ok_or(())?;
        self.offset = end; Ok(value)
    }}
    fn u32(&mut self) -> Result<u32, ()> {{ Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?)) }}
    fn u64(&mut self) -> Result<u64, ()> {{ Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| ())?)) }}
    fn bounded_bytes(&mut self, max: usize) -> Result<&'a [u8], ()> {{ let length = self.u32()? as usize; if length > max {{ return Err(()); }} self.take(length) }}
}}

{decoder}

#[no_mangle]
pub extern "C" fn minijam_refine() -> RefineOutput {{
    let mut input_size = 0usize;
    let status = unsafe {{ minijam_payload(INPUT.as_mut_ptr(), 1048576, &mut input_size) }};
    if status != 0 {{ return error_output(1); }}
    let input = unsafe {{ core::slice::from_raw_parts(INPUT.as_ptr(), input_size) }};
    match refine_payload(input) {{ Ok(size) => RefineOutput {{ data: unsafe {{ OUTPUT.as_ptr() }}, size }}, Err(code) => error_output(code) }}
}}

#[no_mangle]
pub extern "C" fn minijam_accumulate(init_input: *const u8, init_size: usize) {{
    let init_input = unsafe {{ core::slice::from_raw_parts(init_input, init_size) }};
    let mut init_offset = 0usize;
    let authoritative_tick = match read_fnencode(init_input, &mut init_offset) {{ Ok(value) => value, Err(_) => return }};
    let count = unsafe {{ minijam_result_count() }};
    for index in 0..count {{
        let mut size = 0usize;
        if unsafe {{ minijam_result(index, RESULT.as_mut_ptr(), 1048704, &mut size) }} != 0 {{ continue; }}
        let refined = unsafe {{ core::slice::from_raw_parts(RESULT.as_ptr(), size) }};
        let Ok(action) = jamscript_runtime_core::decode_refined_action(refined) else {{ continue; }};
        if jamscript_runtime_core::check_expiry(action.valid_until, authoritative_tick).is_err() {{ continue; }}
        let nonce_key = jamscript_runtime_core::state_key(SERVICE_ID, jamscript_runtime_core::NONCE_SCHEMA_V1, &action.sender);
        let mut nonce_bytes = [0u8; 8];
        let mut nonce_size = 0usize;
        let read_status = unsafe {{ minijam_storage_read(nonce_key.as_ptr(), nonce_key.len(), nonce_bytes.as_mut_ptr(), nonce_bytes.len(), &mut nonce_size) }};
        let expected = match read_status {{ 1 => 0, 0 if nonce_size == 8 => u64::from_le_bytes(nonce_bytes), 0 => accumulate_failure(), _ => accumulate_failure() }};
        if action.nonce != expected {{ continue; }}
        let Ok(score_bytes) = action.result.try_into() else {{ continue; }};
        let score = u64::from_le_bytes(score_bytes);
        {commit}
        let Some(next) = expected.checked_add(1) else {{ continue; }};
        let next_bytes = next.to_le_bytes();
        if unsafe {{ minijam_storage_write(nonce_key.as_ptr(), nonce_key.len(), next_bytes.as_ptr(), next_bytes.len()) }} != 0 {{ accumulate_failure(); }}
    }}
}}

fn read_fnencode(input: &[u8], offset: &mut usize) -> Result<u64, ()> {{
    let first = *input.get(*offset).ok_or(())?; *offset += 1;
    if first < 0x80 {{ return Ok(first as u64); }}
    let mut length = 0usize; while length < 8 && (first & (0x80u8 >> length)) != 0 {{ length += 1; }}
    if length == 0 || length > 7 || input.len().saturating_sub(*offset) < length {{ return Err(()); }}
    let mut low = 0u64; for index in 0..length {{ low |= (*input.get(*offset + index).ok_or(())? as u64) << (8 * index); }}
    *offset += length; Ok(((first as u64 & (0x7fu64 >> length)) << (8 * length)) | low)
}}
fn accumulate_failure() -> ! {{ loop {{ core::hint::spin_loop(); }} }}

fn refine_payload(input: &[u8]) -> Result<usize, u32> {{ {auth_setup} }}
fn error_output(code: u32) -> RefineOutput {{ unsafe {{ OUTPUT[..4].copy_from_slice(&code.to_le_bytes()); RefineOutput {{ data: OUTPUT.as_ptr(), size: 4 }} }} }}
"##,
        service_id = context.service_id,
        genesis_hash = byte_array_literal(&context.genesis_hash),
        selector = byte_array_literal(&selector),
        native_declarations = native_declarations,
        decoder = decoder,
        auth_setup = auth_setup,
        commit = commit,
    ))
}

fn payload_decoder(action: &ActionIr) -> Result<String, String> {
    let mut fields = Vec::new();
    let mut reads = Vec::new();
    for (index, field) in action.input.iter().enumerate() {
        let variable = format!("field_{index}");
        fields.push(variable.clone());
        match field.ty {
            TypeIr::U64 => reads.push(format!("let {variable} = reader.u64()?;")),
            TypeIr::Bytes { max } => reads.push(format!(
                "let {variable} = reader.bounded_bytes({max}usize)?;"
            )),
            _ => {
                return Err(format!(
                    "unsupported action input type for `{}`",
                    field.name
                ))
            }
        }
    }
    let tuple_types = action
        .input
        .iter()
        .map(|field| match field.ty {
            TypeIr::U64 => "u64",
            TypeIr::Bytes { .. } => "&[u8]",
            _ => "()",
        })
        .collect::<Vec<_>>()
        .join(", ");
    let tuple_values = fields.join(", ");
    let tuple_types = if action.input.len() == 1 {
        format!("{tuple_types},")
    } else {
        tuple_types
    };
    let tuple_values = if action.input.len() == 1 {
        format!("{tuple_values},")
    } else {
        tuple_values
    };
    Ok(format!("fn decode_input(input: &[u8]) -> Result<({tuple_types}), ()> {{ let mut reader = PayloadReader {{ input, offset: 0 }}; {reads} if reader.offset != input.len() {{ return Err(()); }} Ok(({tuple_values})) }}", reads = reads.join(" ")))
}

fn compute_setup(
    action: &ActionIr,
    decoded: &str,
    native_imports: &[jamscript_ir::NativeImportIr],
) -> Result<String, String> {
    let field_index = |name: &str| {
        action
            .input
            .iter()
            .position(|field| field.name == name)
            .ok_or_else(|| format!("compute references unknown input field `{name}`"))
    };
    match &action.compute {
        ComputeIr::ReturnInputField { field } => {
            Ok(format!("let value = {decoded}.{};", field_index(field)?))
        }
        ComputeIr::AddInputField { field, value } => Ok(format!(
            "let value = {decoded}.{}.checked_add({value}u128 as u64).ok_or(1u32)?;",
            field_index(field)?
        )),
        ComputeIr::ReturnInteger { value } => Ok(format!("let value: u64 = {value}u128 as u64;")),
        ComputeIr::NativeBytesToU64 {
            module,
            function,
            field,
        } => {
            let index = field_index(field)?;
            let import = native_imports
                .iter()
                .find(|item| item.module == *module && item.function == *function)
                .ok_or_else(|| "native import is missing from IR".to_string())?;
            let symbol = native_symbol(&import.module, &import.function);
            Ok(format!("let mut value = 0u64; let native_status = unsafe {{ {symbol}({decoded}.{index}.as_ptr(), {decoded}.{index}.len() as u32, &mut value) }}; if native_status != 0 {{ if native_status >= NATIVE_ERROR_BASE {{ return Err(NATIVE_ERROR_BASE); }} return Err(NATIVE_ERROR_BASE | native_status); }}"))
        }
        ComputeIr::Unsupported(message) => Err(message.clone()),
    }
}

fn auth_setup(action: &ActionIr, setup: &str) -> String {
    let decode = "let decoded = decode_input(input).map_err(|_| 1u32)?;";
    match action.auth {
        AuthKind::Public => format!("{decode} let computed: Result<u64, u32> = (|| {{ {setup} Ok(value) }})(); let value = computed?; let bytes = value.to_le_bytes(); unsafe {{ OUTPUT[..8].copy_from_slice(&bytes); }} Ok(8)"),
        AuthKind::Wallet => format!("let signed = jamscript_runtime_core::decode_signed_action(input).map_err(|error| error as u32)?; let verified = jamscript_runtime_core::verify_signed_action(signed, GENESIS_HASH, SERVICE_ID, ACTION_SELECTOR).map_err(|error| error as u32)?; let input = verified.payload; {decode} let computed: Result<u64, u32> = (|| {{ {setup} Ok(value) }})(); let value = computed?; let bytes = value.to_le_bytes(); let size = jamscript_runtime_core::encode_refined_action(&verified, &bytes, unsafe {{ &mut OUTPUT }}).map_err(|error| error as u32)?; Ok(size)"),
    }
}

fn accumulate_commit(action: &ActionIr, ir: &ServiceIr) -> Result<String, String> {
    let Some(commit) = &action.commit else {
        return Ok(String::new());
    };
    let state_name = match commit {
        CommitIr::StateSet { state, .. } | CommitIr::StateMax { state, .. } => state,
    };
    let state = ir
        .states
        .iter()
        .find(|state| state.name == *state_name)
        .ok_or_else(|| format!("commit references unknown state `{state_name}`"))?;
    let schema = byte_array_literal(state.schema.as_bytes());
    match commit {
        CommitIr::StateSet { .. } => Ok(format!("let state_key = jamscript_runtime_core::state_key(SERVICE_ID, &{schema}, &action.sender); let value = score.to_le_bytes(); if unsafe {{ minijam_storage_write(state_key.as_ptr(), state_key.len(), value.as_ptr(), value.len()) }} != 0 {{ accumulate_failure(); }}")),
        CommitIr::StateMax { .. } => Ok(format!("let state_key = jamscript_runtime_core::state_key(SERVICE_ID, &{schema}, &action.sender); let mut old_bytes = [0u8; 8]; let mut old_size = 0usize; let state_status = unsafe {{ minijam_storage_read(state_key.as_ptr(), state_key.len(), old_bytes.as_mut_ptr(), old_bytes.len(), &mut old_size) }}; let should_write = match state_status {{ 1 => true, 0 if old_size == 8 => score > u64::from_le_bytes(old_bytes), 0 => accumulate_failure(), _ => accumulate_failure() }}; if should_write {{ let value = score.to_le_bytes(); if unsafe {{ minijam_storage_write(state_key.as_ptr(), state_key.len(), value.as_ptr(), value.len()) }} != 0 {{ accumulate_failure(); }} }}")),
    }
}

fn native_declarations(ir: &ServiceIr) -> String {
    ir.native_imports
        .iter()
        .map(|item| {
            format!(
                "    fn {}(input: *const u8, input_len: u32, output: *mut u64) -> u32;\n",
                native_symbol(&item.module, &item.function)
            )
        })
        .collect()
}
fn native_symbol(module: &str, function: &str) -> String {
    format!("jamscript_native_{module}_{function}_v1")
}
fn byte_array_literal(bytes: &[u8]) -> String {
    format!(
        "[{}]",
        bytes
            .iter()
            .map(|byte| byte.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_ir::{ActionIr, FieldIr, NativeImportIr, TypeIr};
    #[test]
    fn emits_canonical_bounded_bytes_and_native_abi() {
        let source = generate_no_std_rust(&ServiceIr {
            package_name: "x".into(),
            package_version: "0.1.0".into(),
            states: Vec::new(),
            queries: Vec::new(),
            native_imports: vec![NativeImportIr {
                module: "game".into(),
                function: "replay".into(),
            }],
            actions: vec![ActionIr {
                name: "run".into(),
                auth: AuthKind::Wallet,
                input: vec![FieldIr {
                    name: "payload".into(),
                    ty: TypeIr::Bytes { max: 64 },
                }],
                compute: ComputeIr::NativeBytesToU64 {
                    module: "game".into(),
                    function: "replay".into(),
                    field: "payload".into(),
                },
                commit: None,
            }],
        })
        .unwrap();
        assert!(source.contains("bounded_bytes(64usize)"));
        assert!(source.contains("jamscript_native_game_replay_v1"));
        assert!(source.contains("reader.offset != input.len()"));
    }
}
