use jamscript_ir::{ActionIr, ComputeIr, ServiceIr};

pub fn generate_no_std_rust(ir: &ServiceIr) -> Result<String, String> {
    let action = ir
        .actions
        .first()
        .ok_or_else(|| "IR contains no action".to_string())?;
    let setup = match &action.compute {
        ComputeIr::ReturnInputField { field } => input_field(action, field)?,
        ComputeIr::AddInputField { field, value } => format!(
            "let value = read_u64(&input, {})?.checked_add({value}).ok_or(())?;",
            field_offset(action, field)?
        ),
        ComputeIr::ReturnInteger { value } => format!("let value: u64 = {value}u64;"),
        ComputeIr::Unsupported(message) => return Err(message.clone()),
    };
    Ok(format!(
        r#"#![no_std]
#![allow(static_mut_refs)]

#[repr(C)]
pub struct RefineOutput {{ pub data: *const u8, pub size: usize }}

extern "C" {{
    fn minijam_payload(output: *mut u8, capacity: usize, output_size: *mut usize) -> u32;
}}

static mut INPUT: [u8; 1048576] = [0; 1048576];
static mut OUTPUT: [u8; 16] = [0; 16];

fn read_u64(input: &[u8], offset: usize) -> Result<u64, ()> {{
    if input.len() < offset + 8 {{ return Err(()); }}
    let mut bytes = [0u8; 8];
    unsafe {{
        for index in 0..8 {{ bytes[index] = input.as_ptr().add(offset + index).read(); }}
    }}
    Ok(u64::from_le_bytes(bytes))
}}

#[no_mangle]
pub extern "C" fn minijam_refine() -> RefineOutput {{
    let mut input_size = 0usize;
    let status = unsafe {{ minijam_payload(INPUT.as_mut_ptr(), 1048576, &mut input_size) }};
    if status != 0 {{ return RefineOutput {{ data: core::ptr::null(), size: 0 }}; }}
    let input = unsafe {{ core::slice::from_raw_parts(INPUT.as_ptr(), input_size) }};
    let result: Result<u64, ()> = (|| {{ {setup} Ok(value) }})();
    match result {{
        Ok(value) => {{ let bytes = value.to_le_bytes(); unsafe {{ for index in 0..8 {{ OUTPUT[index] = bytes[index]; }} }} RefineOutput {{ data: unsafe {{ OUTPUT.as_ptr() }}, size: 8 }} }},
        Err(()) => RefineOutput {{ data: core::ptr::null(), size: 0 }},
    }}
}}

#[no_mangle]
pub extern "C" fn minijam_accumulate() {{}}
"#
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

fn input_field(action: &ActionIr, field: &str) -> Result<String, String> {
    Ok(format!(
        "let value = read_u64(&input, {})?;",
        field_offset(action, field)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jamscript_ir::{AuthKind, ComputeIr, FieldIr, ServiceIr, TypeIr};
    #[test]
    fn emits_no_std_exports() {
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
        assert!(source.contains("checked_add(1)"));
    }
}
