use blake2b_simd::Params;
use serde::{Deserialize, Serialize};

pub const LANGUAGE_VERSION: &str = "0.1";
pub const ABI_VERSION: u32 = 1;
pub const ACTION_DOMAIN: &[u8] = b"jamscript/action/v1:";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceIr {
    pub package_name: String,
    pub package_version: String,
    pub actions: Vec<ActionIr>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionIr {
    pub name: String,
    pub auth: AuthKind,
    pub input: Vec<FieldIr>,
    pub compute: ComputeIr,
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
pub enum ComputeIr {
    ReturnInputField { field: String },
    ReturnInteger { value: u128 },
    AddInputField { field: String, value: u128 },
    Unsupported(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Abi {
    pub abi_version: u32,
    pub language_version: String,
    pub package: AbiPackage,
    pub actions: Vec<AbiAction>,
    pub queries: Vec<serde_json::Value>,
    pub types: serde_json::Map<String, serde_json::Value>,
    pub state: Vec<serde_json::Value>,
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
    pub compute_output: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
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
    Abi {
        abi_version: ABI_VERSION,
        language_version: LANGUAGE_VERSION.into(),
        package: AbiPackage {
            name: ir.package_name.clone(),
            version: ir.package_version.clone(),
        },
        actions: ir
            .actions
            .iter()
            .map(|action| AbiAction {
                name: action.name.clone(),
                selector: selector_hex(action_selector(&action.name)),
                auth: match action.auth {
                    AuthKind::Wallet => "wallet",
                    AuthKind::Public => "public",
                }
                .into(),
                input: action
                    .input
                    .iter()
                    .map(|f| AbiField {
                        name: f.name.clone(),
                        ty: f.ty.abi_name(),
                    })
                    .collect(),
                compute_output: match &action.compute {
                    ComputeIr::ReturnInputField { field }
                    | ComputeIr::AddInputField { field, .. } => action
                        .input
                        .iter()
                        .find(|f| f.name == *field)
                        .map(|f| f.ty.abi_name())
                        .unwrap_or_else(|| "unknown".into()),
                    ComputeIr::ReturnInteger { .. } => "u64".into(),
                    ComputeIr::Unsupported(_) => "unknown".into(),
                },
            })
            .collect(),
        queries: Vec::new(),
        types: serde_json::Map::new(),
        state: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selector_is_stable() {
        assert_eq!(action_selector("increment"), action_selector("increment"));
    }
}
