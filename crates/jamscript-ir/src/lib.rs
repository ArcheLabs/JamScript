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
    pub key_type: StateKeyType,
    pub value_type: TypeIr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StateKeyType {
    Address,
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
    U64,
    Address,
    U32,
    U128,
    Bool,
    Bytes { max: u32 },
    String { max: u32 },
    Unsupported(String),
}

impl TypeIr {
    pub fn abi_name(&self) -> String {
        match self {
            Self::U64 => "u64".into(),
            Self::Address => "address".into(),
            Self::U32 => "u32".into(),
            Self::U128 => "u128".into(),
            Self::Bool => "bool".into(),
            Self::Bytes { max } => format!("Bytes<{max}>"),
            Self::String { max } => format!("String<{max}>"),
            Self::Unsupported(name) => name.clone(),
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
    pub execute_output: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiState {
    pub name: String,
    pub schema: String,
    pub kind: String,
    #[serde(rename = "keyType")]
    pub key_type: String,
    #[serde(rename = "valueType")]
    pub value_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiQuery {
    pub name: String,
    pub kind: String,
    pub state: String,
    #[serde(rename = "keyType")]
    pub key_type: String,
    pub output: AbiQueryOutput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiQueryOutput {
    #[serde(rename = "type")]
    pub ty: String,
    pub nullable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiType {
    pub kind: String,
    pub max: Option<u32>,
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

pub fn abi_for(ir: &ServiceIr) -> Abi {
    let mut types = std::collections::BTreeMap::new();
    for ty in ir
        .actions
        .iter()
        .flat_map(|action| action.input.iter().map(|field| &field.ty))
        .chain(ir.states.iter().map(|state| &state.value_type))
    {
        let (kind, max) = match ty {
            TypeIr::U64 => ("u64", None),
            TypeIr::Address => ("address", Some(32)),
            TypeIr::U32 => ("u32", None),
            TypeIr::U128 => ("u128", None),
            TypeIr::Bool => ("bool", None),
            TypeIr::Bytes { max } => ("bytes", Some(*max)),
            TypeIr::String { max } => ("string", Some(*max)),
            TypeIr::Unsupported(name) => (name.as_str(), None),
        };
        types.entry(ty.abi_name()).or_insert_with(|| AbiType {
            kind: kind.into(),
            max,
        });
    }
    types.entry("u64".into()).or_insert_with(|| AbiType {
        kind: "u64".into(),
        max: None,
    });
    types.entry("address".into()).or_insert_with(|| AbiType {
        kind: "address".into(),
        max: Some(32),
    });
    let actions = ir
        .actions
        .iter()
        .map(|action| {
            let auth = match action.auth {
                AuthKind::Wallet => "wallet",
                AuthKind::Public => "public",
            };
            AbiAction {
                name: action.name.clone(),
                selector: selector_hex(action_selector(&action.name)),
                auth: auth.into(),
                input: action
                    .input
                    .iter()
                    .map(|f| AbiField {
                        name: f.name.clone(),
                        ty: f.ty.abi_name(),
                    })
                    .collect(),
                execute_output: abi_output(action),
            }
        })
        .collect();
    Abi {
        abi_version: ABI_VERSION,
        language_version: LANGUAGE_VERSION.into(),
        package: AbiPackage {
            name: ir.package_name.clone(),
            version: ir.package_version.clone(),
        },
        actions,
        queries: ir
            .queries
            .iter()
            .filter_map(|query| {
                let state = ir.states.iter().find(|state| state.name == query.state)?;
                Some(AbiQuery {
                    name: query.name.clone(),
                    kind: "state_get".into(),
                    state: state.name.clone(),
                    key_type: "address".into(),
                    output: AbiQueryOutput {
                        ty: state.value_type.abi_name(),
                        nullable: true,
                    },
                })
            })
            .collect(),
        types,
        state: ir
            .states
            .iter()
            .map(|state| AbiState {
                name: state.name.clone(),
                schema: state.schema.clone(),
                kind: "map".into(),
                key_type: "address".into(),
                value_type: state.value_type.abi_name(),
            })
            .collect(),
    }
}

fn abi_output(action: &ActionIr) -> String {
    match &action.body {
        ActionBodyIr::Execute(execute) => match &execute.operation {
            ExecutionOpIr::ReturnInputField { field }
            | ExecutionOpIr::AddInputField { field, .. } => action
                .input
                .iter()
                .find(|f| f.name == *field)
                .map(|f| f.ty.abi_name())
                .unwrap_or_else(|| "unknown".into()),
            ExecutionOpIr::ReturnInteger { .. } | ExecutionOpIr::NativeBytesToU64 { .. } => {
                "u64".into()
            }
        },
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
        });
        assert_eq!(
            abi.types.get("Bytes<64>"),
            Some(&AbiType {
                kind: "bytes".into(),
                max: Some(64),
            })
        );
    }
}
