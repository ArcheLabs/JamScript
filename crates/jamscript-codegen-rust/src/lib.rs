use jamscript_ir::{
    action_selector, ActionBodyIr, ActionIr, AuthKind, ExecutionOpIr, ServiceIr, StateEffectIr,
    TypeIr,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PortableServiceContext {
    pub service_key: [u8; 32],
    pub genesis_hash: [u8; 32],
    pub diagnostic: bool,
}

pub fn generate_no_std_rust(ir: &ServiceIr) -> Result<String, String> {
    generate_no_std_rust_with_context(ir, PortableServiceContext::default())
}

pub fn generate_no_std_rust_with_context(
    ir: &ServiceIr,
    context: PortableServiceContext,
) -> Result<String, String> {
    generate_no_std_rust_with_backend(ir, context, None)
}

pub fn generate_no_std_rust_with_scriptc_context(
    ir: &ServiceIr,
    context: PortableServiceContext,
) -> Result<String, String> {
    let symbol = ir
        .actions
        .first()
        .and_then(|action| match &action.body {
            ActionBodyIr::ScriptC { symbol, .. } => Some(symbol.as_str()),
            ActionBodyIr::Execute(_) => None,
        })
        .ok_or_else(|| "language 0.2 service does not contain a ScriptC action".to_string())?;
    generate_no_std_rust_with_backend(ir, context, Some(symbol))
}

fn generate_no_std_rust_with_backend(
    ir: &ServiceIr,
    context: PortableServiceContext,
    scriptc_symbol: Option<&str>,
) -> Result<String, String> {
    let application_source = generate_application_rust_with_context(ir, context, scriptc_symbol)?;
    let stage_entry = if context.diagnostic {
        "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:entry\");"
    } else {
        ""
    };
    let stage_payload = if context.diagnostic {
        "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:payload\");"
    } else {
        ""
    };
    let stage_input_decode = if context.diagnostic {
        "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:input-decode\");"
    } else {
        ""
    };
    let stage_refine_return = if context.diagnostic {
        "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:refine-return\");"
    } else {
        ""
    };
    let stage_output_encode = if context.diagnostic {
        "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:output-encode\");"
    } else {
        ""
    };
    let stage_output_return = if context.diagnostic {
        "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:output-return\");"
    } else {
        ""
    };
    let refine_call = if context.diagnostic {
        "{ let mut diagnostic_observer = service_runtime_guest::guest_support::DiagnosticObserver; service_runtime_guest::refine_v2_owned_with_observer(&GeneratedApplication, runtime_input, &mut diagnostic_observer) }"
    } else {
        "service_runtime_guest::refine_v2_owned(&GeneratedApplication, runtime_input)"
    };
    Ok(format!(
        r##"#![no_std]
#![allow(static_mut_refs)]
#[cfg(not(target_env = "polkavm"))]
compile_error!("generated service must be built with the official PolkaVM target");

use service_runtime_core::{{
    ManagedStateCommitmentV1, RuntimeRefineInputV1, RuntimeRefineOutputV2, StateRoot,
    MANAGED_STATE_COMMITMENT_KEY_V1,
}};
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
static mut RESULT: [u8; 2097152] = [0; 2097152];
static mut OUTPUT: [u8; 2097152] = [0; 2097152];

{application_source}

#[no_mangle]
pub extern "C" fn minijam_refine() -> RefineOutput {{
    service_runtime_guest::guest_support::reset_runtime();
    {stage_entry}
    let mut input_size = 0usize;
    let status = unsafe {{ minijam_payload(INPUT.as_mut_ptr(), 1048576, &mut input_size) }};
    if status != 0 {{ return error_output(1); }}
    {stage_payload}
    let input = unsafe {{ core::slice::from_raw_parts(INPUT.as_ptr(), input_size) }};
    let runtime_input = match RuntimeRefineInputV1::decode(input) {{
        Ok(value) => value,
        Err(_) => return error_output(1),
    }};
    {stage_input_decode}
    let output = match {refine_call} {{
        Ok(value) => value,
        Err(error) => return error_output(match error {{
            service_runtime_guest::GuestError::InvalidInput => 1,
            service_runtime_guest::GuestError::State => 2,
            service_runtime_guest::GuestError::Application => 3,
        }}),
    }};
    if output.receipts.len() == 1 {{
        if let Some(error_code) = output.receipts[0].error_code.filter(|code| code & 0x8000_0000 != 0) {{
            service_runtime_guest::guest_support::diagnostic_stage(b"jamscript:native-error-output");
            return error_output(error_code);
        }}
    }}
    {stage_refine_return}
    {stage_output_encode}
    let encoded = match output.encode() {{
        Ok(value) => value,
        Err(_) => return error_output(2),
    }};
    if encoded.len() > 2097152 {{ return error_output(14); }}
    unsafe {{ OUTPUT[..encoded.len()].copy_from_slice(&encoded); }}
    {stage_output_return}
    RefineOutput {{ data: unsafe {{ OUTPUT.as_ptr() }}, size: encoded.len() }}
}}

#[no_mangle]
pub extern "C" fn minijam_accumulate(init_input: *const u8, init_size: usize) {{
    let init_input = unsafe {{ core::slice::from_raw_parts(init_input, init_size) }};
    let mut init_offset = 0usize;
    let authoritative_tick = match read_fnencode(init_input, &mut init_offset) {{ Ok(value) => value, Err(_) => return }};
    let mut current = match read_current_commitment() {{ Ok(root) => root, Err(_) => return }};
    let count = unsafe {{ minijam_result_count() }};
    for index in 0..count {{
        let mut size = 0usize;
        if unsafe {{ minijam_result(index, RESULT.as_mut_ptr(), 2097152, &mut size) }} != 0 {{ continue; }}
        let refined = unsafe {{ core::slice::from_raw_parts(RESULT.as_ptr(), size) }};
        let Ok(header) = RuntimeRefineOutputV2::decode_transition_header(refined) else {{ continue; }};
        if header.parent_root != current {{ continue; }}
        if header.transition_valid_until.is_some_and(|valid_until| authoritative_tick > valid_until) {{ continue; }}
        current = header.new_root;
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

fn read_fnencode(input: &[u8], offset: &mut usize) -> Result<u64, ()> {{
    let first = *input.get(*offset).ok_or(())?;
    *offset += 1;
    if first < 0x80 {{ return Ok(first as u64); }}
    let mut length = 0usize;
    while length < 8 && (first & (0x80u8 >> length)) != 0 {{ length += 1; }}
    if length == 0 || length > 7 || input.len().saturating_sub(*offset) < length {{ return Err(()); }}
    let mut low = 0u64;
    for index in 0..length {{ low |= (*input.get(*offset + index).ok_or(())? as u64) << (8 * index); }}
    *offset += length;
    Ok(((first as u64 & (0x7fu64 >> length)) << (8 * length)) | low)
}}

fn error_output(code: u32) -> RefineOutput {{ unsafe {{ OUTPUT[..4].copy_from_slice(&code.to_le_bytes()); RefineOutput {{ data: OUTPUT.as_ptr(), size: 4 }} }} }}
"##,
    ))
}

pub fn generate_builder_application_rust(
    ir: &ServiceIr,
    mut context: PortableServiceContext,
) -> Result<String, String> {
    context.diagnostic = false;
    let symbol = ir.actions.first().and_then(|action| match &action.body {
        ActionBodyIr::ScriptC { symbol, .. } => Some(symbol.as_str()),
        ActionBodyIr::Execute(_) => None,
    });
    generate_application_rust_with_context(ir, context, symbol)
}

fn generate_application_rust_with_context(
    ir: &ServiceIr,
    context: PortableServiceContext,
    scriptc_symbol: Option<&str>,
) -> Result<String, String> {
    let action = ir
        .actions
        .first()
        .ok_or_else(|| "IR contains no action".to_string())?;
    let selector = action_selector(&action.name);
    let decoder = payload_decoder(action)?;
    let (setup, application_effect, decode_input) = match &action.body {
        ActionBodyIr::Execute(execute) => (
            application_setup(
                &execute.operation,
                action,
                "decoded",
                &ir.native_imports,
                context.diagnostic,
            )?,
            application_state_effect(execute.state_effect.as_ref(), ir)?,
            true,
        ),
        ActionBodyIr::ScriptC {
            symbol,
            state_effect,
            ..
        } => {
            let expected = scriptc_symbol.ok_or_else(|| "ScriptC symbol is missing".to_string())?;
            if expected != symbol {
                return Err("ScriptC symbol does not match action metadata".into());
            }
            let stable_symbol = scriptc_native_symbol(symbol);
            let marker = if context.diagnostic {
                "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:scriptc-compute\");"
            } else {
                ""
            };
            (
                format!("let mut value = 0u64; let status = unsafe {{ {stable_symbol}(input.as_ptr(), input.len() as u32, &mut value) }}; if status != 0 {{ return Err(StateAccessError::ApplicationFailed(status)); }} {marker}"),
                application_state_effect(state_effect.as_ref(), ir)?,
                false,
            )
        }
    };
    let native_declarations = native_declarations(ir);
    let scriptc_declaration = scriptc_symbol
        .map(|symbol| {
            format!(
                "    fn {}(input: *const u8, input_len: u32, output: *mut u64) -> u32;\n",
                scriptc_native_symbol(symbol)
            )
        })
        .unwrap_or_default();
    let application_body = application_body(
        action,
        &setup,
        &application_effect,
        context.diagnostic,
        decode_input,
    );
    Ok(format!(
        r##"mod generated_application_impl {{
use service_runtime_core::{{ServiceApplication, ServiceKeyV1, StateAccessError}};

const SERVICE_KEY: ServiceKeyV1 = ServiceKeyV1::new({service_key});
const GENESIS_HASH: [u8; 32] = {genesis_hash};
const ACTION_SELECTOR: [u8; 8] = {selector};

unsafe extern "C" {{
{native_declarations}{scriptc_declaration}}}

struct PayloadReader<'a> {{ input: &'a [u8], offset: usize }}
#[allow(dead_code)]
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

pub struct GeneratedApplication;
impl ServiceApplication for GeneratedApplication {{
    type Error = StateAccessError;
    #[allow(unused_variables)]
    fn execute(
        &self,
        context: &mut service_runtime_core::ExecutionContext<'_>,
        raw_action: &[u8],
    ) -> Result<(), Self::Error> {{ {application_body} }}
}}
}}
pub use generated_application_impl::GeneratedApplication;
"##,
        service_key = byte_array_literal(&context.service_key),
        genesis_hash = byte_array_literal(&context.genesis_hash),
        selector = byte_array_literal(&selector),
        decoder = decoder,
        application_body = application_body,
        scriptc_declaration = scriptc_declaration,
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
    diagnostic: bool,
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
            let marker = if diagnostic {
                "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:application-native-error\");"
            } else {
                ""
            };
            Ok(format!("let mut value = 0u64; let native_status = unsafe {{ {symbol}({decoded}.{index}.as_ptr(), {decoded}.{index}.len() as u32, &mut value) }}; if native_status != 0 {{ {marker} return Err(StateAccessError::ApplicationFailed(0x8000_0000u32 | native_status)); }}"))
        }
    }
}

fn application_body(
    action: &ActionIr,
    setup: &str,
    effect: &str,
    diagnostic: bool,
    decode_input: bool,
) -> String {
    let marker = |name: &str| {
        if diagnostic {
            format!(
                "service_runtime_guest::guest_support::diagnostic_stage(b\"jamscript:{name}\");"
            )
        } else {
            String::new()
        }
    };
    let auth = match action.auth {
        AuthKind::Public => "let sender = [0u8; 32]; let input = raw_action;".to_string(),
        AuthKind::Wallet => [
            "let signed = jamscript_runtime_core::decode_signed_action_v2(raw_action).map_err(|_| StateAccessError::Backend)?;",
            &marker("application-auth-decoded"),
            &marker("application-auth-verifying"),
            "let verified = jamscript_runtime_core::verify_signed_action_v2(signed, GENESIS_HASH, SERVICE_KEY, ACTION_SELECTOR).map_err(|_| StateAccessError::Backend)?;",
            &marker("application-auth-verified"),
            "let sender = verified.sender;",
            "let nonce_key = jamscript_runtime_core::nonce_key(&sender);",
            &marker("application-nonce-reading"),
            "let nonce_bytes = context.state().get(&nonce_key)?.unwrap_or_default();",
            "let expected_nonce = match nonce_bytes.as_slice() { [] => 0u64, bytes if bytes.len() == 8 => u64::from_le_bytes(bytes.try_into().map_err(|_| StateAccessError::Backend)?), _ => return Err(StateAccessError::Backend) };",
            "if verified.nonce != expected_nonce { return Err(StateAccessError::Backend); }",
            "context.constrain_valid_until(verified.valid_until);",
            "let next_nonce = expected_nonce.checked_add(1).ok_or(StateAccessError::Backend)?;",
            &marker("application-nonce-writing"),
            "context.state().set(&nonce_key, &next_nonce.to_le_bytes())?;",
            &marker("application-nonce-written"),
            "let input = verified.payload;",
        ].join(" ")
    };
    let decode = if decode_input {
        "let decoded = decode_input(input).map_err(|_| StateAccessError::Backend)?;"
    } else {
        ""
    };
    let value = if decode_input {
        format!(
            "let value: u64 = (|| -> Result<u64, StateAccessError> {{ {setup} Ok(value) }})()?;"
        )
    } else {
        setup.to_string()
    };
    format!(
        "{auth} {begin} context.begin_transaction()?; let business = (|| -> Result<(), StateAccessError> {{ {decode} {value} {effect} Ok(()) }})(); {business_done} match business {{ Ok(()) => {{ {commit} context.commit_transaction() }}, Err(StateAccessError::MissingWitness) => {{ context.rollback_transaction()?; Err(StateAccessError::MissingWitness) }}, Err(StateAccessError::InvalidProof) => {{ context.rollback_transaction()?; Err(StateAccessError::InvalidProof) }}, Err(StateAccessError::ApplicationFailed(code)) => {{ let _ = context.rollback_transaction(); Err(StateAccessError::ApplicationFailed(code)) }}, Err(_) => {{ context.rollback_transaction()?; Err(StateAccessError::ApplicationFailed(1)) }} }}",
        begin = marker("application-business"),
        business_done = marker("application-business-done"),
        commit = marker("application-commit"),
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
fn scriptc_native_symbol(action: &str) -> String {
    format!("jamscript_scriptc_{action}_v1")
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
        assert!(source.contains("0x8000_0000u32 | native_status"));
        assert!(source.contains("output.receipts.len() == 1"));
        assert!(source.contains("return error_output(error_code)"));
    }

    #[test]
    fn guest_and_builder_embed_the_same_generated_application() {
        let services = [
            ServiceIr {
                package_name: "counter".into(),
                package_version: "0.1.0".into(),
                states: vec![jamscript_ir::StateIr {
                    name: "counter".into(),
                    schema: "counter/v1".into(),
                    key_type: jamscript_ir::StateKeyType::Address,
                    value_type: TypeIr::U64,
                }],
                queries: Vec::new(),
                native_imports: Vec::new(),
                actions: vec![ActionIr {
                    name: "increment".into(),
                    auth: AuthKind::Wallet,
                    input: vec![FieldIr {
                        name: "value".into(),
                        ty: TypeIr::U64,
                    }],
                    body: ActionBodyIr::Execute(jamscript_ir::ExecuteIr {
                        operation: ExecutionOpIr::ReturnInputField {
                            field: "value".into(),
                        },
                        state_effect: Some(StateEffectIr::Set {
                            state: "counter".into(),
                        }),
                    }),
                }],
            },
            ServiceIr {
                package_name: "game".into(),
                package_version: "0.1.0".into(),
                states: Vec::new(),
                queries: Vec::new(),
                native_imports: vec![NativeImportIr {
                    module: "game".into(),
                    function: "replay".into(),
                }],
                actions: vec![ActionIr {
                    name: "submitRun".into(),
                    auth: AuthKind::Wallet,
                    input: vec![FieldIr {
                        name: "run".into(),
                        ty: TypeIr::Bytes { max: 262_144 },
                    }],
                    body: ActionBodyIr::Execute(jamscript_ir::ExecuteIr {
                        operation: ExecutionOpIr::NativeBytesToU64 {
                            module: "game".into(),
                            function: "replay".into(),
                            field: "run".into(),
                        },
                        state_effect: None,
                    }),
                }],
            },
        ];
        let context = PortableServiceContext {
            service_key: [7; 32],
            genesis_hash: [8; 32],
            diagnostic: false,
        };
        for ir in services {
            let builder = generate_builder_application_rust(&ir, context).unwrap();
            let guest = generate_no_std_rust_with_context(&ir, context).unwrap();
            assert!(guest.contains(&builder));
            assert!(!builder.contains("minijam_payload"));
            assert!(!builder.contains("minijam_storage_write"));
            assert!(!builder.contains("read_fnencode"));
            assert!(!builder.contains("service_runtime_guest"));
        }
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
        assert!(source.contains("RuntimeRefineOutputV2::decode_transition_header"));
        assert!(source.contains("MANAGED_STATE_COMMITMENT_KEY_V1"));
        assert!(source.contains("SERVICE_KEY"));
        assert!(!source.contains("SERVICE_ID"));
        assert!(!source.contains("service_id"));
        assert!(!source.contains("minijam_storage_write(state_key"));
        assert!(!source.contains("decode_refined_action"));

        let accumulate = source
            .split("pub extern \"C\" fn minijam_accumulate")
            .nth(1)
            .and_then(|tail| tail.split("struct GeneratedApplication").next())
            .expect("generated accumulate entrypoint");
        assert_eq!(accumulate.matches("minijam_storage_write").count(), 1);
        assert!(!accumulate.contains("nonce_key"));
        assert!(!accumulate.contains("application_key_v1"));
        assert!(!accumulate.contains("StateMax"));
        assert!(!accumulate.contains("StateSet"));
        assert!(accumulate.contains("authoritative_tick > valid_until"));
        assert!(accumulate.contains("header.parent_root != current"));
        assert!(accumulate.contains("MANAGED_STATE_COMMITMENT_KEY_V1"));
    }

    #[test]
    fn diagnostic_guest_has_fail_fast_traps_and_stage_markers() {
        let ir = ServiceIr {
            package_name: "x".into(),
            package_version: "0.1.0".into(),
            states: Vec::new(),
            queries: Vec::new(),
            native_imports: Vec::new(),
            actions: vec![ActionIr {
                name: "run".into(),
                auth: AuthKind::Wallet,
                input: vec![FieldIr {
                    name: "value".into(),
                    ty: TypeIr::U64,
                }],
                body: ActionBodyIr::Execute(jamscript_ir::ExecuteIr {
                    operation: ExecutionOpIr::AddInputField {
                        field: "value".into(),
                        value: 1,
                    },
                    state_effect: None,
                }),
            }],
        };
        let source = generate_no_std_rust_with_context(
            &ir,
            PortableServiceContext {
                service_key: [1; 32],
                genesis_hash: [2; 32],
                diagnostic: true,
            },
        )
        .unwrap();
        assert!(source.contains("guest_support::diagnostic_stage"));
        assert!(source.contains("jamscript:entry"));
        assert!(source.contains("guest_support::DiagnosticObserver"));
        assert!(source.contains("jamscript:application-auth-verifying"));
        assert!(source.contains("DiagnosticObserver"));
        assert!(!source.contains("global_allocator"));
        assert!(!source.contains("fn panic(_info: &core::panic::PanicInfo<'_>) -> ! { loop {} }"));
    }
}
