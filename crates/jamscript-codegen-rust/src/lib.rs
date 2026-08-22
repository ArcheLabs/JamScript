use jamscript_ir::{
    action_selector, ActionBodyIr, ActionIr, AuthKind, ExecutionOpIr, ServiceIr, StateEffectIr,
    TypeIr,
};

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
    let ActionBodyIr::Execute(execute) = &action.body;
    let setup = application_setup(&execute.operation, action, "decoded", &ir.native_imports)?;
    let native_declarations = native_declarations(ir);
    let application_effect = application_state_effect(execute.state_effect.as_ref(), ir)?;
    let application_body = application_body(action, &setup, &application_effect);
    Ok(format!(
        r##"#![no_std]
#![allow(static_mut_refs)]

use service_runtime_core::{{
    ManagedStateCommitmentV1, RuntimeRefineInputV1, RuntimeRefineOutputV1,
    ServiceApplication, StateAccessError, StateRoot, MANAGED_STATE_COMMITMENT_KEY_V1,
}};
use service_runtime_guest::refine;

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
static mut RUNTIME_HEAP: [u8; 65536] = [0; 65536];
static mut RUNTIME_HEAP_OFFSET: usize = 0;

struct RuntimeAllocator;
unsafe impl core::alloc::GlobalAlloc for RuntimeAllocator {{
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {{
        let base = RUNTIME_HEAP.as_mut_ptr() as usize;
        let offset = (RUNTIME_HEAP_OFFSET + layout.align() - 1) & !(layout.align() - 1);
        let end = offset.saturating_add(layout.size());
        if end > RUNTIME_HEAP.len() {{ return core::ptr::null_mut(); }}
        RUNTIME_HEAP_OFFSET = end;
        base.saturating_add(offset) as *mut u8
    }}
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {{}}
}}
#[global_allocator]
static RUNTIME_ALLOCATOR: RuntimeAllocator = RuntimeAllocator;

const SERVICE_ID: u32 = {service_id};
const GENESIS_HASH: [u8; 32] = {genesis_hash};
const ACTION_SELECTOR: [u8; 8] = {selector};

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
    #[allow(dead_code)]
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
    let runtime_input = match RuntimeRefineInputV1::decode(input) {{
        Ok(value) => value,
        Err(_) => return error_output(1),
    }};
    let output = match refine(&GeneratedApplication, &runtime_input) {{
        Ok(value) => value,
        Err(error) => return error_output(match error {{
            service_runtime_guest::GuestError::InvalidInput => 1,
            service_runtime_guest::GuestError::State => 2,
            service_runtime_guest::GuestError::Application => 3,
        }}),
    }};
    let encoded = match output.encode() {{
        Ok(value) => value,
        Err(_) => return error_output(2),
    }};
    if encoded.len() > 1048704 {{ return error_output(14); }}
    unsafe {{ OUTPUT[..encoded.len()].copy_from_slice(&encoded); }}
    RefineOutput {{ data: unsafe {{ OUTPUT.as_ptr() }}, size: encoded.len() }}
}}

#[no_mangle]
pub extern "C" fn minijam_accumulate(init_input: *const u8, init_size: usize) {{
    let _ = (init_input, init_size);
    let mut current = match read_current_commitment() {{ Ok(root) => root, Err(_) => return }};
    let count = unsafe {{ minijam_result_count() }};
    for index in 0..count {{
        let mut size = 0usize;
        if unsafe {{ minijam_result(index, RESULT.as_mut_ptr(), 1048704, &mut size) }} != 0 {{ continue; }}
        let refined = unsafe {{ core::slice::from_raw_parts(RESULT.as_ptr(), size) }};
        let Ok(output) = RuntimeRefineOutputV1::decode(refined) else {{ continue; }};
        if output.parent_root == current {{ current = output.new_root; }}
    }}
    if current != read_current_commitment().unwrap_or(current) {{
        let commitment = ManagedStateCommitmentV1::new(current).encode();
        let key = MANAGED_STATE_COMMITMENT_KEY_V1;
        let _ = unsafe {{
            minijam_storage_write(key.as_ptr(), key.len(), commitment.as_ptr(), commitment.len())
        }};
    }}
}}

fn read_current_commitment() -> Result<StateRoot, ()> {{
    let key = MANAGED_STATE_COMMITMENT_KEY_V1;
    let mut bytes = [0u8; 34];
    let mut size = 0usize;
    let status = unsafe {{
        minijam_storage_read(key.as_ptr(), key.len(), bytes.as_mut_ptr(), bytes.len(), &mut size)
    }};
    match status {{
        1 => Ok(service_runtime_core::EMPTY_STATE_ROOT_V1),
        0 if size == bytes.len() => ManagedStateCommitmentV1::decode(&bytes)
            .map(|commitment| commitment.root)
            .map_err(|_| ()),
        _ => Err(()),
    }}
}}

struct GeneratedApplication;
impl ServiceApplication for GeneratedApplication {{
    type Error = StateAccessError;
    fn execute(
        &self,
        context: &mut service_runtime_core::ExecutionContext<'_>,
        raw_action: &[u8],
    ) -> Result<(), Self::Error> {{ {application_body} }}
}}

fn error_output(code: u32) -> RefineOutput {{ unsafe {{ OUTPUT[..4].copy_from_slice(&code.to_le_bytes()); RefineOutput {{ data: OUTPUT.as_ptr(), size: 4 }} }} }}
"##,
        service_id = context.service_id,
        genesis_hash = byte_array_literal(&context.genesis_hash),
        selector = byte_array_literal(&selector),
        native_declarations = native_declarations,
        decoder = decoder,
        application_body = application_body,
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

fn application_setup(
    operation: &ExecutionOpIr,
    action: &ActionIr,
    decoded: &str,
    native_imports: &[jamscript_ir::NativeImportIr],
) -> Result<String, String> {
    let field_index = |name: &str| {
        action
            .input
            .iter()
            .position(|field| field.name == name)
            .ok_or_else(|| format!("execute references unknown input field `{name}`"))
    };
    match operation {
        ExecutionOpIr::ReturnInputField { field } => {
            Ok(format!("let value = {decoded}.{};", field_index(field)?))
        }
        ExecutionOpIr::AddInputField { field, value } => Ok(format!(
            "let value = {decoded}.{}.checked_add({value}u128 as u64).ok_or(StateAccessError::Backend)?;",
            field_index(field)?
        )),
        ExecutionOpIr::ReturnInteger { value } => {
            Ok(format!("let value: u64 = {value}u128 as u64;"))
        }
        ExecutionOpIr::NativeBytesToU64 {
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
            Ok(format!("let mut value = 0u64; let native_status = unsafe {{ {symbol}({decoded}.{index}.as_ptr(), {decoded}.{index}.len() as u32, &mut value) }}; if native_status != 0 {{ return Err(StateAccessError::Backend); }}"))
        }
    }
}

fn application_body(action: &ActionIr, setup: &str, effect: &str) -> String {
    let auth = match action.auth {
        AuthKind::Public => "let sender = [0u8; 32]; let input = raw_action;".to_string(),
        AuthKind::Wallet => [
            "let signed = jamscript_runtime_core::decode_signed_action(raw_action).map_err(|_| StateAccessError::Backend)?;",
            "let verified = jamscript_runtime_core::verify_signed_action(signed, GENESIS_HASH, SERVICE_ID, ACTION_SELECTOR).map_err(|_| StateAccessError::Backend)?;",
            "let sender = verified.sender;",
            "let nonce_key = jamscript_runtime_core::nonce_key(&sender);",
            "let nonce_bytes = context.state().get(&nonce_key)?.unwrap_or_default();",
            "let expected_nonce = match nonce_bytes.as_slice() { [] => 0u64, bytes if bytes.len() == 8 => u64::from_le_bytes(bytes.try_into().map_err(|_| StateAccessError::Backend)?), _ => return Err(StateAccessError::Backend) };",
            "if verified.nonce != expected_nonce { return Err(StateAccessError::Backend); }",
            "let next_nonce = expected_nonce.checked_add(1).ok_or(StateAccessError::Backend)?;",
            "context.state().set(&nonce_key, &next_nonce.to_le_bytes())?;",
            "let input = verified.payload;",
        ].join(" ")
    };
    format!(
        "{auth} context.begin_transaction()?; let business = (|| -> Result<(), StateAccessError> {{ let decoded = decode_input(input).map_err(|_| StateAccessError::Backend)?; let value: u64 = (|| -> Result<u64, StateAccessError> {{ {setup} Ok(value) }})()?; {effect} Ok(()) }})(); match business {{ Ok(()) => context.commit_transaction(), Err(StateAccessError::MissingWitness) => {{ context.rollback_transaction()?; Err(StateAccessError::MissingWitness) }}, Err(StateAccessError::InvalidProof) => {{ context.rollback_transaction()?; Err(StateAccessError::InvalidProof) }}, Err(_) => {{ context.rollback_transaction()?; Err(StateAccessError::ApplicationFailed(1)) }} }}"
    )
}

fn application_state_effect(
    effect: Option<&StateEffectIr>,
    ir: &ServiceIr,
) -> Result<String, String> {
    let Some(effect) = effect else {
        return Ok(String::new());
    };
    let state_name = match effect {
        StateEffectIr::Set { state } | StateEffectIr::Max { state } => state,
    };
    let state = ir
        .states
        .iter()
        .find(|state| state.name == *state_name)
        .ok_or_else(|| format!("execute references unknown state {state_name}"))?;
    let schema = byte_array_literal(state.schema.as_bytes());
    match effect {
        StateEffectIr::Set { .. } => Ok(format!(
            "let state_key = service_runtime_core::application_key_v1(&{schema}, &sender).map_err(|_| StateAccessError::Backend)?; context.state().set(&state_key, &value.to_le_bytes())?;"
        )),
        StateEffectIr::Max { .. } => Ok(format!(
            "let state_key = service_runtime_core::application_key_v1(&{schema}, &sender).map_err(|_| StateAccessError::Backend)?; let should_write = match context.state().get(&state_key)? {{ None => true, Some(bytes) => {{ if bytes.len() != 8 {{ return Err(StateAccessError::Backend); }} let old = u64::from_le_bytes(bytes.try_into().map_err(|_| StateAccessError::Backend)?); value > old }} }}; if should_write {{ context.state().set(&state_key, &value.to_le_bytes())?; }}"
        )),
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
                body: ActionBodyIr::Execute(jamscript_ir::ExecuteIr {
                    operation: ExecutionOpIr::NativeBytesToU64 {
                        module: "game".into(),
                        function: "replay".into(),
                        field: "payload".into(),
                    },
                    state_effect: None,
                }),
            }],
        })
        .unwrap();
        assert!(source.contains("bounded_bytes(64usize)"));
        assert!(source.contains("jamscript_native_game_replay_v1"));
        assert!(source.contains("reader.offset != input.len()"));
    }

    #[test]
    fn keeps_application_state_in_refine_and_accumulate_only_cas_commitment() {
        let source = generate_no_std_rust(&ServiceIr {
            package_name: "x".into(),
            package_version: "0.1.0".into(),
            states: vec![jamscript_ir::StateIr {
                name: "score".into(),
                schema: "score/v1".into(),
                key_type: jamscript_ir::StateKeyType::Address,
                value_type: TypeIr::U64,
            }],
            queries: Vec::new(),
            native_imports: Vec::new(),
            actions: vec![ActionIr {
                name: "set".into(),
                auth: AuthKind::Wallet,
                input: vec![FieldIr {
                    name: "score".into(),
                    ty: TypeIr::U64,
                }],
                body: ActionBodyIr::Execute(jamscript_ir::ExecuteIr {
                    operation: ExecutionOpIr::ReturnInputField {
                        field: "score".into(),
                    },
                    state_effect: Some(StateEffectIr::Max {
                        state: "score".into(),
                    }),
                }),
            }],
        })
        .unwrap();
        assert!(source.contains("context.state().get"));
        assert!(source.contains("RuntimeRefineOutputV1::decode"));
        assert!(source.contains("MANAGED_STATE_COMMITMENT_KEY_V1"));
        assert!(!source.contains("minijam_storage_write(state_key"));
        assert!(!source.contains("decode_refined_action"));
    }
}
