use blake2b_simd::Params;
use serde::{Deserialize, Serialize};

pub const LANGUAGE_VERSION: &str = "0.1";
pub const ABI_VERSION: u32 = 1;
pub const ACTION_DOMAIN: &[u8] = b"jamscript/action/v1:";
pub const MAX_ACTION_PAYLOAD_BYTES: u32 = 1_000_000;
pub const NATIVE_ABI_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceIr {
    pub package_name: String,
    pub package_version: String,
    /// Original TypeScript compilation unit retained for the opt-in ScriptC
    /// backend. Legacy consumers continue to use the structured IR fields.
    #[serde(default)]
    pub source: String,
    pub states: Vec<StateIr>,
    pub actions: Vec<ActionIr>,
    pub queries: Vec<QueryIr>,
    pub native_imports: Vec<NativeImportIr>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionIr {
    pub name: String,
    pub auth: AuthKind,
    pub input: Vec<FieldIr>,
    pub body: ActionBodyIr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateIr {
    pub name: String,
    pub schema: String,
    pub kind: StateKind,
    pub key_type: TypeIr,
    pub value_type: TypeIr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StateKind {
    Scalar,
    Map,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryIr {
    pub name: String,
    pub state: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeImportIr {
    pub module: String,
    pub function: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActionBodyIr {
    Execute(ExecuteIr),
    /// Language 0.2 keeps service metadata in this IR but delegates the
    /// compute body to an explicitly compiled ScriptC symbol.
    ScriptC {
        symbol: String,
        source_unit: String,
        state_effect: Option<StateEffectIr>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecuteIr {
    pub operation: ExecutionOpIr,
    pub state_effect: Option<StateEffectIr>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExecutionOpIr {
    ReturnInputField {
        field: String,
    },
    ReturnInteger {
        value: u128,
    },
    AddInputField {
        field: String,
        value: u128,
    },
    NativeBytesToU64 {
        module: String,
        function: String,
        field: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StateEffectIr {
    Set { state: String },
    Max { state: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AuthKind {
    Wallet,
    Public,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FieldIr {
    pub name: String,
    pub ty: TypeIr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TypeIr {
    Unit,
    U8,
    U16,
    U64,
    U32,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    Bool,
    Address,
    FixedBytes { len: u32 },
    Bytes { max: u32 },
    String { max: u32 },
    FixedArray { item: Box<TypeIr>, len: u32 },
    Array { item: Box<TypeIr>, max: u32 },
    Option { item: Box<TypeIr> },
    Tuple { items: Vec<TypeIr> },
    Record { fields: Vec<FieldIr> },
    Enum { variants: Vec<VariantIr> },
    Result { ok: Box<TypeIr>, err: Box<TypeIr> },
    Unsupported(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VariantIr {
    pub name: String,
    pub index: u8,
    pub ty: TypeIr,
}

impl TypeIr {
    pub fn abi_name(&self) -> String {
        match self {
            Self::Unit => "unit".into(),
            Self::U8 => "u8".into(),
            Self::U16 => "u16".into(),
            Self::U64 => "u64".into(),
            Self::U32 => "u32".into(),
            Self::U128 => "u128".into(),
            Self::I8 => "i8".into(),
            Self::I16 => "i16".into(),
            Self::I32 => "i32".into(),
            Self::I64 => "i64".into(),
            Self::I128 => "i128".into(),
            Self::Bool => "bool".into(),
            Self::Address => "address".into(),
            Self::FixedBytes { len } => format!("FixedBytes<{len}>"),
            Self::Bytes { max } => format!("Bytes<{max}>"),
            Self::String { max } => format!("String<{max}>"),
            Self::FixedArray { item, len } => format!("FixedArray<{}, {len}>", item.abi_name()),
            Self::Array { item, max } => format!("Array<{}, {max}>", item.abi_name()),
            Self::Option { item } => format!("Option<{}>", item.abi_name()),
            Self::Tuple { items } => format!(
                "Tuple<{}>",
                items
                    .iter()
                    .map(TypeIr::abi_name)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Record { fields } => format!(
                "Record<{}>",
                fields
                    .iter()
                    .map(|field| format!("{}:{}", field.name, field.ty.abi_name()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Enum { variants } => format!(
                "Enum<{}>",
                variants
                    .iter()
                    .map(|variant| format!(
                        "{}={}:{}",
                        variant.name,
                        variant.index,
                        variant.ty.abi_name()
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Result { ok, err } => format!("Result<{},{}>", ok.abi_name(), err.abi_name()),
            Self::Unsupported(name) => name.clone(),
        }
    }

    /// Maximum canonical JAM encoding length.  A bounded dynamic value uses
    /// the same general-natural prefix as `jam_codec::Compact`.
    pub fn max_encoded_len(&self) -> Result<usize, &'static str> {
        fn add(left: usize, right: usize) -> Result<usize, &'static str> {
            left.checked_add(right).ok_or("encoded length overflow")
        }
        fn mul(left: usize, right: usize) -> Result<usize, &'static str> {
            left.checked_mul(right).ok_or("encoded length overflow")
        }
        fn compact_len(value: u128) -> usize {
            if value <= u64::MAX as u128 {
                let value = value as u64;
                if value < (1u64 << 56) {
                    1 + if value == 0 {
                        0
                    } else {
                        ((64 - value.leading_zeros()) as usize - 1) / 7
                    }
                } else {
                    9
                }
            } else {
                compact_len(value & u64::MAX as u128) + compact_len(value >> 64)
            }
        }
        match self {
            Self::Unit => Ok(0),
            Self::U8 | Self::I8 | Self::Bool => Ok(1),
            Self::U16 | Self::I16 => Ok(2),
            Self::U32 | Self::I32 => Ok(4),
            Self::U64 | Self::I64 => Ok(8),
            Self::U128 | Self::I128 => Ok(16),
            Self::Address => Ok(32),
            Self::FixedBytes { len } => Ok(*len as usize),
            Self::Bytes { max } | Self::String { max } => {
                add(compact_len(*max as u128), *max as usize)
            }
            Self::FixedArray { item, len } => mul(item.max_encoded_len()?, *len as usize),
            Self::Array { item, max } => add(
                compact_len(*max as u128),
                mul(item.max_encoded_len()?, *max as usize)?,
            ),
            Self::Option { item } => add(1, item.max_encoded_len()?),
            Self::Tuple { items } => items
                .iter()
                .try_fold(0, |total, item| add(total, item.max_encoded_len()?)),
            Self::Record { fields } => fields
                .iter()
                .try_fold(0, |total, field| add(total, field.ty.max_encoded_len()?)),
            Self::Enum { variants } => variants
                .iter()
                .map(|variant| variant.ty.max_encoded_len())
                .try_fold(1usize, |max, value| Ok(max.max(1 + value?))),
            Self::Result { ok, err } => Ok(1 + ok.max_encoded_len()?.max(err.max_encoded_len()?)),
            Self::Unsupported(_) => Err("unsupported ABI type"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Abi {
    pub abi_version: u32,
    pub language_version: String,
    pub package: AbiPackage,
    pub actions: Vec<AbiAction>,
    pub queries: Vec<AbiQuery>,
    pub types: std::collections::BTreeMap<String, AbiType>,
    pub state: Vec<AbiState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiPackage {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiAction {
    pub name: String,
    pub selector: String,
    pub auth: String,
    pub input: Vec<AbiField>,
    #[serde(rename = "executeOutput")]
    pub execute_output: AbiTypeDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: AbiTypeDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiState {
    pub name: String,
    pub schema: String,
    pub kind: String,
    #[serde(rename = "keyType")]
    pub key_type: AbiTypeDescriptor,
    #[serde(rename = "valueType")]
    pub value_type: AbiTypeDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiQuery {
    pub name: String,
    pub kind: String,
    pub state: String,
    #[serde(rename = "keyType")]
    pub key_type: AbiTypeDescriptor,
    pub output: AbiQueryOutput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiQueryOutput {
    #[serde(rename = "type")]
    pub ty: AbiTypeDescriptor,
    pub nullable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiType {
    pub kind: String,
    pub max: Option<u32>,
    pub descriptor: AbiTypeDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbiError {
    UnsupportedType(String),
}

impl std::fmt::Display for AbiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(name) => write!(formatter, "unsupported ABI type `{name}`"),
        }
    }
}

impl std::error::Error for AbiError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AbiTypeDescriptor {
    Unit,
    Bool,
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    Address,
    FixedBytes { len: u32 },
    Bytes { max: u32 },
    String { max: u32 },
    FixedArray { item: Box<Self>, len: u32 },
    Array { item: Box<Self>, max: u32 },
    Option { item: Box<Self> },
    Tuple { items: Vec<Self> },
    Record { fields: Vec<AbiField> },
    Enum { variants: Vec<AbiVariant> },
    Result { ok: Box<Self>, err: Box<Self> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiVariant {
    pub name: String,
    pub index: u8,
    #[serde(rename = "type")]
    pub ty: AbiTypeDescriptor,
}

impl TryFrom<&TypeIr> for AbiTypeDescriptor {
    type Error = AbiError;

    fn try_from(ty: &TypeIr) -> Result<Self, Self::Error> {
        Ok(match ty {
            TypeIr::Unit => Self::Unit,
            TypeIr::Bool => Self::Bool,
            TypeIr::U8 => Self::U8,
            TypeIr::U16 => Self::U16,
            TypeIr::U32 => Self::U32,
            TypeIr::U64 => Self::U64,
            TypeIr::U128 => Self::U128,
            TypeIr::I8 => Self::I8,
            TypeIr::I16 => Self::I16,
            TypeIr::I32 => Self::I32,
            TypeIr::I64 => Self::I64,
            TypeIr::I128 => Self::I128,
            TypeIr::Address => Self::Address,
            TypeIr::FixedBytes { len } => Self::FixedBytes { len: *len },
            TypeIr::Bytes { max } => Self::Bytes { max: *max },
            TypeIr::String { max } => Self::String { max: *max },
            TypeIr::FixedArray { item, len } => Self::FixedArray {
                item: Box::new(Self::try_from(item.as_ref())?),
                len: *len,
            },
            TypeIr::Array { item, max } => Self::Array {
                item: Box::new(Self::try_from(item.as_ref())?),
                max: *max,
            },
            TypeIr::Option { item } => Self::Option {
                item: Box::new(Self::try_from(item.as_ref())?),
            },
            TypeIr::Tuple { items } => Self::Tuple {
                items: items.iter().map(Self::try_from).collect::<Result<_, _>>()?,
            },
            TypeIr::Record { fields } => Self::Record {
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok(AbiField {
                            name: field.name.clone(),
                            ty: Self::try_from(&field.ty)?,
                        })
                    })
                    .collect::<Result<_, AbiError>>()?,
            },
            TypeIr::Enum { variants } => Self::Enum {
                variants: variants
                    .iter()
                    .map(|variant| {
                        Ok(AbiVariant {
                            name: variant.name.clone(),
                            index: variant.index,
                            ty: Self::try_from(&variant.ty)?,
                        })
                    })
                    .collect::<Result<_, AbiError>>()?,
            },
            TypeIr::Result { ok, err } => Self::Result {
                ok: Box::new(Self::try_from(ok.as_ref())?),
                err: Box::new(Self::try_from(err.as_ref())?),
            },
            TypeIr::Unsupported(name) => return Err(AbiError::UnsupportedType(name.clone())),
        })
    }
}

pub fn action_selector(name: &str) -> [u8; 8] {
    let mut input = Vec::with_capacity(ACTION_DOMAIN.len() + name.len());
    input.extend_from_slice(ACTION_DOMAIN);
    input.extend_from_slice(name.as_bytes());
    let digest = Params::new().hash_length(32).hash(&input);
    let mut selector = [0u8; 8];
    selector.copy_from_slice(&digest.as_bytes()[..8]);
    selector
}

pub fn selector_hex(selector: [u8; 8]) -> String {
    format!(
        "0x{}",
        selector
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

pub fn abi_for(ir: &ServiceIr) -> Result<Abi, AbiError> {
    abi_for_language(ir, LANGUAGE_VERSION)
}

pub fn abi_for_language(ir: &ServiceIr, language_version: &str) -> Result<Abi, AbiError> {
    let mut types = std::collections::BTreeMap::new();
    for ty in ir
        .actions
        .iter()
        .flat_map(|action| action.input.iter().map(|field| &field.ty))
        .chain(
            ir.states
                .iter()
                .flat_map(|state| [&state.key_type, &state.value_type]),
        )
    {
        collect_abi_types(&mut types, ty)?;
    }
    types.entry("u64".into()).or_insert_with(|| AbiType {
        kind: "u64".into(),
        max: None,
        descriptor: AbiTypeDescriptor::U64,
    });
    types.entry("address".into()).or_insert_with(|| AbiType {
        kind: "address".into(),
        max: Some(32),
        descriptor: AbiTypeDescriptor::Address,
    });
    let actions = ir
        .actions
        .iter()
        .map(|action| -> Result<AbiAction, AbiError> {
            let auth = match action.auth {
                AuthKind::Wallet => "wallet",
                AuthKind::Public => "public",
            };
            Ok(AbiAction {
                name: action.name.clone(),
                selector: selector_hex(action_selector(&action.name)),
                auth: auth.into(),
                input: action
                    .input
                    .iter()
                    .map(|f| {
                        Ok(AbiField {
                            name: f.name.clone(),
                            ty: AbiTypeDescriptor::try_from(&f.ty)?,
                        })
                    })
                    .collect::<Result<Vec<_>, AbiError>>()?,
                execute_output: abi_output(action)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Abi {
        abi_version: ABI_VERSION,
        language_version: language_version.into(),
        package: AbiPackage {
            name: ir.package_name.clone(),
            version: ir.package_version.clone(),
        },
        actions,
        queries: ir
            .queries
            .iter()
            .map(|query| {
                let state = ir
                    .states
                    .iter()
                    .find(|state| state.name == query.state)
                    .ok_or_else(|| {
                        AbiError::UnsupportedType(format!("unknown query state `{}`", query.state))
                    })?;
                Ok(AbiQuery {
                    name: query.name.clone(),
                    kind: "state_get".into(),
                    state: state.name.clone(),
                    key_type: AbiTypeDescriptor::try_from(&state.key_type)?,
                    output: AbiQueryOutput {
                        ty: AbiTypeDescriptor::try_from(&state.value_type)?,
                        nullable: true,
                    },
                })
            })
            .collect::<Result<Vec<_>, AbiError>>()?,
        types,
        state: ir
            .states
            .iter()
            .map(|state| -> Result<AbiState, AbiError> {
                Ok(AbiState {
                    name: state.name.clone(),
                    schema: state.schema.clone(),
                    kind: match state.kind {
                        StateKind::Scalar => "scalar",
                        StateKind::Map => "map",
                    }
                    .into(),
                    key_type: AbiTypeDescriptor::try_from(&state.key_type)?,
                    value_type: AbiTypeDescriptor::try_from(&state.value_type)?,
                })
            })
            .collect::<Result<Vec<_>, AbiError>>()?,
    })
}

fn collect_abi_types(
    types: &mut std::collections::BTreeMap<String, AbiType>,
    ty: &TypeIr,
) -> Result<(), AbiError> {
    let (kind, max) = abi_kind_max(ty);
    let descriptor = AbiTypeDescriptor::try_from(ty)?;
    types.entry(ty.abi_name()).or_insert_with(|| AbiType {
        kind: kind.into(),
        max,
        descriptor,
    });
    match ty {
        TypeIr::FixedArray { item, .. } | TypeIr::Array { item, .. } | TypeIr::Option { item } => {
            collect_abi_types(types, item)?
        }
        TypeIr::Tuple { items } => {
            for item in items {
                collect_abi_types(types, item)?
            }
        }
        TypeIr::Record { fields } => {
            for field in fields {
                collect_abi_types(types, &field.ty)?
            }
        }
        TypeIr::Enum { variants } => {
            for variant in variants {
                collect_abi_types(types, &variant.ty)?
            }
        }
        TypeIr::Result { ok, err } => {
            collect_abi_types(types, ok)?;
            collect_abi_types(types, err)?;
        }
        _ => {}
    }
    Ok(())
}

fn abi_kind_max(ty: &TypeIr) -> (&str, Option<u32>) {
    match ty {
        TypeIr::Unit => ("unit", None),
        TypeIr::U8 => ("u8", None),
        TypeIr::U16 => ("u16", None),
        TypeIr::U32 => ("u32", None),
        TypeIr::U64 => ("u64", None),
        TypeIr::U128 => ("u128", None),
        TypeIr::I8 => ("i8", None),
        TypeIr::I16 => ("i16", None),
        TypeIr::I32 => ("i32", None),
        TypeIr::I64 => ("i64", None),
        TypeIr::I128 => ("i128", None),
        TypeIr::Bool => ("bool", None),
        TypeIr::Address => ("address", Some(32)),
        TypeIr::FixedBytes { len } => ("fixedBytes", Some(*len)),
        TypeIr::Bytes { max } => ("bytes", Some(*max)),
        TypeIr::String { max } => ("string", Some(*max)),
        TypeIr::FixedArray { .. } => ("fixedArray", None),
        TypeIr::Array { max, .. } => ("array", Some(*max)),
        TypeIr::Option { .. } => ("option", None),
        TypeIr::Tuple { .. } => ("tuple", None),
        TypeIr::Record { .. } => ("record", None),
        TypeIr::Enum { .. } => ("enum", None),
        TypeIr::Result { .. } => ("result", None),
        TypeIr::Unsupported(name) => (name.as_str(), None),
    }
}

fn abi_output(action: &ActionIr) -> Result<AbiTypeDescriptor, AbiError> {
    match &action.body {
        ActionBodyIr::Execute(execute) => match &execute.operation {
            ExecutionOpIr::ReturnInputField { field }
            | ExecutionOpIr::AddInputField { field, .. } => {
                let input = action
                    .input
                    .iter()
                    .find(|input| input.name == *field)
                    .ok_or_else(|| {
                        AbiError::UnsupportedType(format!("unknown action output field `{field}`"))
                    })?;
                AbiTypeDescriptor::try_from(&input.ty)
            }
            ExecutionOpIr::ReturnInteger { .. } | ExecutionOpIr::NativeBytesToU64 { .. } => {
                Ok(AbiTypeDescriptor::U64)
            }
        },
        ActionBodyIr::ScriptC { .. } => Ok(AbiTypeDescriptor::Unit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_is_stable() {
        assert_eq!(action_selector("increment"), action_selector("increment"));
    }

    #[test]
    fn abi_emits_bounded_bytes_descriptor() {
        let abi = abi_for(&ServiceIr {
            package_name: "game".into(),
            package_version: "0.1.0".into(),
            source: String::new(),
            states: Vec::new(),
            actions: vec![ActionIr {
                name: "submit".into(),
                auth: AuthKind::Wallet,
                input: vec![FieldIr {
                    name: "run".into(),
                    ty: TypeIr::Bytes { max: 64 },
                }],
                body: ActionBodyIr::Execute(ExecuteIr {
                    operation: ExecutionOpIr::ReturnInteger { value: 1 },
                    state_effect: None,
                }),
            }],
            queries: Vec::new(),
            native_imports: Vec::new(),
        })
        .unwrap();
        assert_eq!(
            abi.types.get("Bytes<64>"),
            Some(&AbiType {
                kind: "bytes".into(),
                max: Some(64),
                descriptor: AbiTypeDescriptor::Bytes { max: 64 },
            })
        );
    }

    #[test]
    fn unsupported_types_cannot_cross_abi_boundary() {
        let ir = ServiceIr {
            package_name: "future".into(),
            package_version: "0.1.0".into(),
            source: String::new(),
            states: Vec::new(),
            actions: vec![ActionIr {
                name: "submit".into(),
                auth: AuthKind::Wallet,
                input: vec![FieldIr {
                    name: "future".into(),
                    ty: TypeIr::Unsupported("future-type".into()),
                }],
                body: ActionBodyIr::Execute(ExecuteIr {
                    operation: ExecutionOpIr::ReturnInteger { value: 1 },
                    state_effect: None,
                }),
            }],
            queries: Vec::new(),
            native_imports: Vec::new(),
        };
        assert_eq!(
            abi_for(&ir),
            Err(AbiError::UnsupportedType("future-type".into()))
        );
    }

    #[test]
    fn scalar_and_map_states_have_distinct_abi_kinds() {
        let ir = ServiceIr {
            package_name: "states".into(),
            package_version: "0.1.0".into(),
            source: String::new(),
            states: vec![
                StateIr {
                    name: "config".into(),
                    schema: "config/v1".into(),
                    kind: StateKind::Scalar,
                    key_type: TypeIr::Unit,
                    value_type: TypeIr::U64,
                },
                StateIr {
                    name: "scores".into(),
                    schema: "scores/v1".into(),
                    kind: StateKind::Map,
                    key_type: TypeIr::Address,
                    value_type: TypeIr::U64,
                },
            ],
            actions: Vec::new(),
            queries: Vec::new(),
            native_imports: Vec::new(),
        };
        let abi = abi_for(&ir).unwrap();
        assert_eq!(abi.state[0].kind, "scalar");
        assert_eq!(abi.state[1].kind, "map");
    }
}
