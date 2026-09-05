use jamscript_ir::{
    ActionBodyIr, ActionIr, AuthKind, FieldIr, NativeImportIr, QueryIr, ServiceIr, StateEffectIr,
    StateIr, StateKind, TypeIr, VariantIr, MAX_ACTION_PAYLOAD_BYTES,
};
use swc_common::{sync::Lrc, FileName, SourceMap};
use swc_ecma_ast::*;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax, TsSyntax};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("source parse failed: {0}")]
    Syntax(String),
    #[error("error[JAM{code}]: {message}")]
    Diagnostic { code: &'static str, message: String },
}

pub fn parse_service(
    source: &str,
    package_name: &str,
    package_version: &str,
) -> Result<ServiceIr, ParseError> {
    parse_service_v02(source, package_name, package_version, &[])
}

pub fn parse_service_v02(
    source: &str,
    package_name: &str,
    package_version: &str,
    native_modules: &[String],
) -> Result<ServiceIr, ParseError> {
    parse_service_formal(source, package_name, package_version, native_modules)
}

fn parse_service_formal(
    source: &str,
    package_name: &str,
    package_version: &str,
    native_modules: &[String],
) -> Result<ServiceIr, ParseError> {
    let cm: Lrc<SourceMap> = Default::default();
    let file = cm.new_source_file(
        Lrc::new(FileName::Custom("service.ts".into())),
        source.to_owned(),
    );
    let lexer = Lexer::new(
        Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: false,
            dts: false,
            no_early_errors: true,
            disallow_ambiguous_jsx_like: false,
        }),
        EsVersion::Es2022,
        StringInput::from(&*file),
        None,
    );
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|error| ParseError::Syntax(format!("{:?}", error.kind())))?;
    if let Some(error) = parser.take_errors().into_iter().next() {
        return Err(ParseError::Syntax(format!("{:?}", error.kind())));
    }

    let mut actions = Vec::new();
    let mut states = Vec::new();
    let mut queries = Vec::new();
    let mut native_imports = Vec::new();
    let mut aliases = std::collections::BTreeMap::new();
    for item in module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                collect_import(&import, native_modules, &mut native_imports)?
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
                decl: Decl::Fn(_),
                ..
            })) => {
                // Exported helpers are retained in the original source unit
                // and are not part of the JamScript metadata IR.
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                let Decl::Var(var) = export.decl else {
                    return Err(diag(
                        "1002",
                        "only `export const` declarations are supported",
                    ));
                };
                if var.kind != VarDeclKind::Const {
                    return Err(diag(
                        "1003",
                        "only `export const` declarations are supported",
                    ));
                }
                for declarator in var.decls {
                    let Pat::Ident(binding) = declarator.name else {
                        return Err(diag("1004", "destructuring declarations are not supported"));
                    };
                    let Some(init) = declarator.init else {
                        return Err(diag("1005", "exported declaration needs an initializer"));
                    };
                    match call_name(&init).as_deref() {
                        Some("action") => actions.push(parse_scriptc_action(
                            binding.id.sym.as_ref(),
                            &init,
                            &aliases,
                        )?),
                        Some("query") => queries.push(parse_query(binding.id.sym.as_ref(), &init)?),
                        _ => return Err(diag("1002", "exports must be action(...) or query(...)")),
                    }
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var))) if var.kind == VarDeclKind::Const => {
                for declarator in var.decls {
                    let Pat::Ident(binding) = declarator.name else {
                        return Err(diag("1004", "destructuring declarations are not supported"));
                    };
                    let Some(init) = declarator.init else {
                        return Err(diag("1031", "state declaration needs an initializer"));
                    };
                    if is_type_constructor(&init) {
                        aliases.insert(binding.id.sym.to_string(), parse_type(&init, &aliases)?);
                    } else {
                        states.push(parse_state(binding.id.sym.as_ref(), &init, &aliases)?);
                    }
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(_))) => {
                // ScriptC owns the original compilation unit. The Rust parser only
                // extracts JamScript metadata, so top-level helpers remain in
                // `ServiceIr::source` and are deliberately opaque here.
            }
            ModuleItem::Stmt(Stmt::Empty(..)) => {}
            _ => {
                return Err(diag(
                    "1001",
                    "only imports, state declarations, and exported actions/queries are supported",
                ))
            }
        }
    }
    if native_imports
        .iter()
        .map(|item| (&item.module, &item.function))
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != native_imports.len()
    {
        return Err(diag("1032", "duplicate native imports are not supported"));
    }
    validate_service(&states, &actions, &queries)?;
    Ok(ServiceIr {
        package_name: package_name.into(),
        package_version: package_version.into(),
        source: source.into(),
        states,
        actions,
        queries,
        native_imports,
    })
}

fn collect_import(
    import: &ImportDecl,
    native_modules: &[String],
    native_imports: &mut Vec<NativeImportIr>,
) -> Result<(), ParseError> {
    let source = import.src.value.to_string();
    let native_module = source.strip_prefix("native:");
    if source != "jam" && native_module.is_none() {
        return Err(diag(
            "1007",
            "Service imports must come from `jam` or `native:<module>`",
        ));
    }
    if let Some(module) = native_module {
        if module.is_empty()
            || !is_c_identifier(module)
            || !native_modules.iter().any(|m| m == module)
        {
            return Err(diag(
                "1033",
                format!("unknown or invalid native module `{module}`"),
            ));
        }
        for specifier in &import.specifiers {
            let ImportSpecifier::Named(named) = specifier else {
                return Err(diag("1008", "native imports must be named imports"));
            };
            let function = match &named.imported {
                Some(ModuleExportName::Ident(ident)) => ident.sym.to_string(),
                Some(ModuleExportName::Str(string)) => string.value.to_string(),
                None => named.local.sym.to_string(),
            };
            if named.local.sym != function || !is_c_identifier(&function) {
                return Err(diag(
                    "1034",
                    "native imports must use identifier-safe names",
                ));
            }
            native_imports.push(NativeImportIr {
                module: module.into(),
                function,
            });
        }
        return Ok(());
    }
    for specifier in &import.specifiers {
        let name = match specifier {
            ImportSpecifier::Named(named) => match &named.imported {
                Some(ModuleExportName::Ident(ident)) => ident.sym.to_string(),
                Some(ModuleExportName::Str(string)) => string.value.to_string(),
                None => named.local.sym.to_string(),
            },
            ImportSpecifier::Default(_) | ImportSpecifier::Namespace(_) => {
                return Err(diag(
                    "1008",
                    "default and namespace imports are not supported",
                ))
            }
        };
        if name != "abort"
            && !matches!(
                name.as_str(),
                "action"
                    | "wallet"
                    | "publicAction"
                    | "unit"
                    | "bool"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "bytes"
                    | "string"
                    | "fixedBytes"
                    | "fixedArray"
                    | "array"
                    | "option"
                    | "tuple"
                    | "record"
                    | "enumType"
                    | "result"
                    | "address"
                    | "state"
                    | "stateMap"
                    | "query"
            )
        {
            return Err(diag(
                "1009",
                format!("`{name}` is not part of the JamScript 0.2 standard library"),
            ));
        }
    }
    Ok(())
}

fn parse_scriptc_action(
    name: &str,
    init: &Expr,
    aliases: &std::collections::BTreeMap<String, TypeIr>,
) -> Result<ActionIr, ParseError> {
    let config = call_object(init, "action", "1110")?;
    let mut auth = None;
    let mut input = None;
    let mut has_execute = false;
    for prop in &config.props {
        let PropOrSpread::Prop(prop) = prop else {
            return Err(diag("1111", "spread properties are not supported"));
        };
        match &**prop {
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("auth") => {
                auth = Some(parse_auth(&kv.value)?);
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("input") => {
                input = Some(parse_input(&kv.value, aliases)?);
            }
            Prop::Method(method) if key_name(&method.key).as_deref() == Some("execute") => {
                has_execute = true;
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("execute") => {
                let Expr::Fn(function) = &*kv.value else {
                    return Err(diag("1112", "execute must be a function"));
                };
                let _ = function;
                has_execute = true;
            }
            _ => return Err(diag("1113", "unsupported action property for ScriptC")),
        }
    }
    let input = input.ok_or_else(|| diag("1114", "action must declare an input object"))?;
    validate_input(&input)?;
    if !has_execute {
        return Err(diag("1115", "action must declare execute(ctx, input)"));
    }
    Ok(ActionIr {
        name: name.into(),
        auth: auth.ok_or_else(|| diag("1116", "action must declare auth"))?,
        input,
        body: ActionBodyIr::ScriptC {
            symbol: name.into(),
            source_unit: "service.ts".into(),
            state_effect: None,
        },
    })
}

fn parse_auth(expr: &Expr) -> Result<AuthKind, ParseError> {
    let name =
        call_name(expr).ok_or_else(|| diag("1019", "auth must be wallet() or publicAction()"))?;
    let Expr::Call(call) = expr else {
        unreachable!()
    };
    if !call.args.is_empty() {
        return Err(diag("1019", "auth must be wallet() or publicAction()"));
    }
    match name.as_str() {
        "wallet" => Ok(AuthKind::Wallet),
        "publicAction" => Ok(AuthKind::Public),
        _ => Err(diag("1019", "auth must be wallet() or publicAction()")),
    }
}

fn parse_input(
    expr: &Expr,
    aliases: &std::collections::BTreeMap<String, TypeIr>,
) -> Result<Vec<FieldIr>, ParseError> {
    let Expr::Object(object) = expr else {
        return Err(diag("1020", "input must be an object schema"));
    };
    object
        .props
        .iter()
        .map(|prop| {
            let PropOrSpread::Prop(prop) = prop else {
                return Err(diag("1021", "input spread properties are not supported"));
            };
            let Prop::KeyValue(kv) = &**prop else {
                return Err(diag("1022", "input fields must be `name: Type` entries"));
            };
            let name = key_name(&kv.key)
                .ok_or_else(|| diag("1022", "input field names must be identifiers"))?;
            Ok(FieldIr {
                name,
                ty: parse_type(&kv.value, aliases)?,
            })
        })
        .collect()
}

fn parse_type(
    expr: &Expr,
    aliases: &std::collections::BTreeMap<String, TypeIr>,
) -> Result<TypeIr, ParseError> {
    if let Expr::Ident(name) = expr {
        if let Some(alias) = aliases.get(name.sym.as_ref()) {
            return Ok(alias.clone());
        }
        return match name.sym.as_ref() {
            "unit" => Ok(TypeIr::Unit),
            "bool" => Ok(TypeIr::Bool),
            "u8" => Ok(TypeIr::U8),
            "u16" => Ok(TypeIr::U16),
            "u64" => Ok(TypeIr::U64),
            "u32" => Ok(TypeIr::U32),
            "u128" => Ok(TypeIr::U128),
            "i8" => Ok(TypeIr::I8),
            "i16" => Ok(TypeIr::I16),
            "i32" => Ok(TypeIr::I32),
            "i64" => Ok(TypeIr::I64),
            "i128" => Ok(TypeIr::I128),
            "address" => Ok(TypeIr::Address),
            other => Err(diag("1023", format!("unsupported ABI type `{other}`"))),
        };
    }
    let Expr::Call(call) = expr else {
        return Err(diag("1023", "unsupported ABI type expression"));
    };
    let Callee::Expr(callee) = &call.callee else {
        return Err(diag("1023", "unsupported ABI type expression"));
    };
    let Expr::Ident(name) = &**callee else {
        return Err(diag("1023", "unsupported ABI type expression"));
    };
    let name = name.sym.as_ref();
    let bound = |index: usize, label: &str| -> Result<u32, ParseError> {
        let value = call
            .args
            .get(index)
            .and_then(|arg| integer_literal(&arg.expr).ok().flatten())
            .ok_or_else(|| diag("1023", format!("{label} requires an integer bound")))?;
        if value == 0 || value > MAX_ACTION_PAYLOAD_BYTES as u128 {
            return Err(diag(
                "1036",
                format!("{label} bound must be between 1 and {MAX_ACTION_PAYLOAD_BYTES}"),
            ));
        }
        Ok(value as u32)
    };
    match name {
        "fixedBytes" if call.args.len() == 1 => Ok(TypeIr::FixedBytes {
            len: bound(0, "fixedBytes(N)")?,
        }),
        "bytes" if call.args.len() == 1 => Ok(TypeIr::Bytes {
            max: bound(0, "bytes(N)")?,
        }),
        "string" if call.args.len() == 1 => Ok(TypeIr::String {
            max: bound(0, "string(N)")?,
        }),
        "fixedArray" if call.args.len() == 2 => Ok(TypeIr::FixedArray {
            item: Box::new(parse_type(&call.args[0].expr, aliases)?),
            len: bound(1, "fixedArray(T,N)")?,
        }),
        "array" if call.args.len() == 2 => Ok(TypeIr::Array {
            item: Box::new(parse_type(&call.args[0].expr, aliases)?),
            max: bound(1, "array(T,N)")?,
        }),
        "option" if call.args.len() == 1 => Ok(TypeIr::Option {
            item: Box::new(parse_type(&call.args[0].expr, aliases)?),
        }),
        "tuple" => Ok(TypeIr::Tuple {
            items: call
                .args
                .iter()
                .map(|arg| parse_type(&arg.expr, aliases))
                .collect::<Result<_, _>>()?,
        }),
        "record" if call.args.len() == 1 => Ok(TypeIr::Record {
            fields: parse_input(&call.args[0].expr, aliases)?,
        }),
        "enumType" if call.args.len() == 1 => parse_enum(&call.args[0].expr, aliases),
        "result" if call.args.len() == 2 => Ok(TypeIr::Result {
            ok: Box::new(parse_type(&call.args[0].expr, aliases)?),
            err: Box::new(parse_type(&call.args[1].expr, aliases)?),
        }),
        _ => Err(diag("1023", "unsupported ABI type expression")),
    }
}

fn parse_enum(
    expr: &Expr,
    aliases: &std::collections::BTreeMap<String, TypeIr>,
) -> Result<TypeIr, ParseError> {
    let Expr::Object(object) = expr else {
        return Err(diag("1023", "enumType requires a variant object"));
    };
    if object.props.len() > 256 {
        return Err(diag("1030", "enumType supports at most 256 variants"));
    }
    let mut variants = Vec::new();
    for (index, prop) in object.props.iter().enumerate() {
        let PropOrSpread::Prop(prop) = prop else {
            return Err(diag("1023", "enum variants do not support spreads"));
        };
        let Prop::KeyValue(kv) = &**prop else {
            return Err(diag("1023", "enum variants must be name: Type entries"));
        };
        variants.push(VariantIr {
            name: key_name(&kv.key)
                .ok_or_else(|| diag("1023", "enum variant names must be identifiers"))?,
            index: index as u8,
            ty: parse_type(&kv.value, aliases)?,
        });
    }
    Ok(TypeIr::Enum { variants })
}

fn is_type_constructor(expr: &Expr) -> bool {
    matches!(
        call_name(expr).as_deref(),
        Some(
            "fixedBytes"
                | "bytes"
                | "string"
                | "fixedArray"
                | "array"
                | "option"
                | "tuple"
                | "record"
                | "enumType"
                | "result"
        )
    )
}

fn parse_state(
    name: &str,
    init: &Expr,
    aliases: &std::collections::BTreeMap<String, TypeIr>,
) -> Result<StateIr, ParseError> {
    let call =
        call_name(init).ok_or_else(|| diag("1041", "state must be state(...) or stateMap(...)"))?;
    if call != "state" && call != "stateMap" {
        return Err(diag("1041", "state must be state(...) or stateMap(...)"));
    }
    let object = call_object(init, &call, "1041")?;
    let mut schema = None;
    let mut key_type = None;
    let mut value_type = None;
    for prop in &object.props {
        let PropOrSpread::Prop(prop) = prop else {
            return Err(diag("1042", "stateMap spread properties are not supported"));
        };
        let Prop::KeyValue(kv) = &**prop else {
            return Err(diag("1042", "stateMap fields must be key/value entries"));
        };
        match key_name(&kv.key).as_deref() {
            Some("schema") => schema = Some(string_literal(&kv.value)?),
            Some("key") => key_type = Some(parse_type(&kv.value, aliases)?),
            Some("value") => value_type = Some(parse_type(&kv.value, aliases)?),
            _ => return Err(diag("1043", "state supports schema, key, and value only")),
        }
    }
    let schema = schema.ok_or_else(|| diag("1044", "stateMap requires schema"))?;
    if schema.is_empty() || schema.len() > 64 || !schema.is_ascii() {
        return Err(diag(
            "1045",
            "state schema must be non-empty ASCII of at most 64 bytes",
        ));
    }
    let value_type = value_type.ok_or_else(|| diag("1047", "state requires value"))?;
    let key_type = if call == "state" {
        if key_type.is_some() {
            return Err(diag("1046", "scalar state does not accept a key"));
        }
        TypeIr::Unit
    } else {
        key_type.ok_or_else(|| diag("1046", "stateMap requires key"))?
    };
    let value_max = value_type
        .max_encoded_len()
        .map_err(|error| diag("1055", error))?;
    if value_max > service_runtime_core::MAX_STATE_VALUE_BYTES {
        return Err(diag(
            "1057",
            format!("state `{name}` maximum encoded value length exceeds Managed State limit"),
        ));
    }
    let key_max = key_type
        .max_encoded_len()
        .map_err(|error| diag("1058", error))?;
    let final_key_max = 1usize
        .checked_add(2)
        .and_then(|length| length.checked_add(schema.len()))
        .and_then(|length| length.checked_add(key_max))
        .ok_or_else(|| diag("1058", "state key encoded length overflows compiler limits"))?;
    if final_key_max > service_runtime_core::MAX_STATE_KEY_BYTES {
        return Err(diag(
            "1058",
            format!("state `{name}` maximum encoded key length exceeds Managed State limit"),
        ));
    }
    Ok(StateIr {
        name: name.into(),
        schema,
        kind: if call == "state" {
            StateKind::Scalar
        } else {
            StateKind::Map
        },
        key_type,
        value_type,
    })
}

fn parse_query(name: &str, init: &Expr) -> Result<QueryIr, ParseError> {
    let Expr::Call(call) = init else {
        return Err(diag("1049", "query must be query(state)"));
    };
    if call_name(init).as_deref() != Some("query") || call.args.len() != 1 {
        return Err(diag("1049", "query must be query(state)"));
    }
    let Expr::Ident(state) = &*call.args[0].expr else {
        return Err(diag("1050", "query argument must be a state map"));
    };
    Ok(QueryIr {
        name: name.into(),
        state: state.sym.to_string(),
    })
}

fn validate_service(
    states: &[StateIr],
    actions: &[ActionIr],
    queries: &[QueryIr],
) -> Result<(), ParseError> {
    let names = states
        .iter()
        .map(|state| state.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if names.len() != states.len() {
        return Err(diag("1051", "state variable names must be unique"));
    }
    if states
        .iter()
        .map(|state| state.schema.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != states.len()
    {
        return Err(diag("1052", "state schemas must be unique"));
    }
    if actions
        .iter()
        .map(|action| jamscript_ir::action_selector(&action.name))
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != actions.len()
    {
        return Err(diag("1056", "action selectors must be unique"));
    }
    for action in actions {
        let effect = match &action.body {
            ActionBodyIr::Execute(execute) => execute.state_effect.as_ref(),
            ActionBodyIr::ScriptC { state_effect, .. } => state_effect.as_ref(),
        };
        if let Some(effect) = effect {
            let state = match effect {
                StateEffectIr::Set { state } | StateEffectIr::Max { state } => state,
            };
            if !names.contains(state.as_str()) {
                return Err(diag(
                    "1053",
                    format!("execute references unknown state `{state}`"),
                ));
            }
        }
    }
    for query in queries {
        if !names.contains(query.state.as_str()) {
            return Err(diag(
                "1054",
                format!("query references unknown state `{}`", query.state),
            ));
        }
    }
    Ok(())
}

fn validate_input(input: &[FieldIr]) -> Result<(), ParseError> {
    if input
        .iter()
        .map(|field| field.name.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != input.len()
    {
        return Err(diag("1055", "input field names must be unique"));
    }
    let total = input
        .iter()
        .try_fold(0usize, |total, field| {
            let size = field.ty.max_encoded_len().map_err(|_| ())?;
            total.checked_add(size).ok_or(())
        })
        .map_err(|_| diag("1056", "action input exceeds the bounded payload limit"))?;
    if total > MAX_ACTION_PAYLOAD_BYTES as usize {
        return Err(diag(
            "1056",
            "action input exceeds the bounded payload limit",
        ));
    }
    Ok(())
}

fn call_object<'a>(
    expr: &'a Expr,
    expected: &str,
    code: &'static str,
) -> Result<&'a ObjectLit, ParseError> {
    let Expr::Call(call) = expr else {
        return Err(diag(
            code,
            format!("must be initialized with {expected}(... )"),
        ));
    };
    if call_name(expr).as_deref() != Some(expected) || call.args.len() != 1 {
        return Err(diag(
            code,
            format!("must be initialized with {expected}(... )"),
        ));
    }
    let Expr::Object(object) = &*call.args[0].expr else {
        return Err(diag(code, "configuration must be an object literal"));
    };
    Ok(object)
}
fn call_name(expr: &Expr) -> Option<String> {
    let Expr::Call(call) = expr else { return None };
    let Callee::Expr(callee) = &call.callee else {
        return None;
    };
    let Expr::Ident(name) = &**callee else {
        return None;
    };
    Some(name.sym.to_string())
}
fn string_literal(expr: &Expr) -> Result<String, ParseError> {
    let Expr::Lit(Lit::Str(value)) = expr else {
        return Err(diag("1045", "schema must be a string literal"));
    };
    Ok(value.value.to_string())
}
fn integer_literal(expr: &Expr) -> Result<Option<u128>, ParseError> {
    let Expr::Lit(Lit::Num(value)) = expr else {
        return Ok(None);
    };
    let raw = value
        .raw
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.value.to_string());
    let normalized = raw.replace('_', "");
    if normalized.contains('.')
        || normalized.contains('e')
        || normalized.contains('E')
        || value.value < 0.0
    {
        return Err(diag(
            "1029",
            "only non-negative integer literals are supported",
        ));
    }
    let parsed = normalized
        .parse::<u128>()
        .map_err(|_| diag("1029", "invalid integer literal"))?;
    if parsed > 9_007_199_254_740_991 {
        return Err(diag(
            "1030",
            "integer literals above Number.MAX_SAFE_INTEGER are not supported",
        ));
    }
    Ok(Some(parsed))
}
fn key_name(key: &PropName) -> Option<String> {
    if let PropName::Ident(ident) = key {
        Some(ident.sym.to_string())
    } else {
        None
    }
}
fn is_c_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
fn diag(code: &'static str, message: impl Into<String>) -> ParseError {
    ParseError::Diagnostic {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_bytes() {
        let source = r#"import { action, wallet, bytes } from "jam"; export const add = action({ auth: wallet(), input: { value: bytes(0) }, execute(ctx, input) { return input.value; } });"#;
        assert!(parse_service(source, "x", "0.1.0").is_err());
    }

    #[test]
    fn enforces_managed_state_encoded_limits() {
        let valid = r#"import { action, wallet, stateMap, string, bytes, address } from "jam"; const x = stateMap({ schema: "x/v1", key: string(32), value: bytes(1024) }); export const set = action({ auth: wallet(), input: { value: bytes(1) }, execute(ctx, input) { return input.value; } });"#;
        assert!(parse_service(valid, "x", "0.1.0").is_ok());

        let oversized_value = r#"import { action, wallet, stateMap, bytes, address } from "jam"; const x = stateMap({ schema: "x/v1", key: address, value: bytes(65536) }); export const set = action({ auth: wallet(), input: { value: bytes(1) }, execute(ctx, input) { return input.value; } });"#;
        let error = parse_service(oversized_value, "x", "0.1.0").unwrap_err();
        assert!(error.to_string().contains("maximum encoded value length"));

        let oversized_key = r#"import { action, wallet, stateMap, fixedBytes, bytes } from "jam"; const x = stateMap({ schema: "x/v1", key: fixedBytes(1000000), value: bytes(1) }); export const set = action({ auth: wallet(), input: { value: bytes(1) }, execute(ctx, input) { return input.value; } });"#;
        let error = parse_service(oversized_key, "x", "0.1.0").unwrap_err();
        assert!(error.to_string().contains("maximum encoded key length"));
    }

    #[test]
    fn parses_scalar_state_with_empty_unit_key() {
        let source = r#"import { action, wallet, state, query, u64, bytes } from "jam"; const config = state({ schema: "config/v1", value: u64 }); export const set = action({ auth: wallet(), input: { value: bytes(1) }, execute(ctx, input) { return input.value; } }); export const get = query(config);"#;
        let ir = parse_service(source, "config", "0.1.0").unwrap();
        assert_eq!(ir.states[0].kind, StateKind::Scalar);
        assert_eq!(ir.states[0].key_type, TypeIr::Unit);
    }

    #[test]
    fn parses_multi_action_typed_state_metadata() {
        let source = r#"
            import { action, wallet, stateMap, query, bytes, address, record, u32 } from "jam";
            const Key = bytes(32);
            const Entry = record({ owner: address, value: u32 });
            const entries = stateMap({ schema: "test.entries/v1", key: Key, value: Entry });
            export const create = action({ auth: wallet(), input: { key: Key, value: u32 }, execute(ctx, input) {} });
            export const update = action({ auth: wallet(), input: { key: Key, value: u32 }, execute(ctx, input) {} });
            export const getEntry = query(entries);
        "#;
        let ir = parse_service_v02(source, "typed-state-fixture", "1.0.0", &[]).unwrap();
        assert_eq!(ir.actions.len(), 2);
        assert_eq!(ir.queries.len(), 1);
        assert_eq!(ir.states[0].kind, jamscript_ir::StateKind::Map);
        assert_eq!(ir.states[0].key_type, TypeIr::Bytes { max: 32 });
        assert!(matches!(ir.states[0].value_type, TypeIr::Record { .. }));
    }

    #[test]
    fn parses_scriptc_wallet_action() {
        let source = r#"import { action, wallet, u64 } from "jam"; export const increment = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) { return input.value + 1; } });"#;
        let ir = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap();
        assert_eq!(ir.actions[0].auth, AuthKind::Wallet);
        assert_eq!(ir.source, source);
        assert!(
            matches!(ir.actions[0].body, ActionBodyIr::ScriptC { ref symbol, .. } if symbol == "increment")
        );
    }

    #[test]
    fn parses_declared_scriptc_native_import() {
        let source = r#"import { action, wallet, bytes, stateMap, address, u32 } from "jam";
            import { calculate } from "native:math";
            const result = stateMap({ schema: "native/result/v1", key: address, value: u32 });
            export const run = action({ auth: wallet(), input: { payload: bytes(32) }, execute(ctx, input) { result.set(ctx.sender, calculate(input.payload)); } });"#;
        let ir = parse_service_v02(source, "native", "0.2.0", &["math".into()]).unwrap();
        assert_eq!(
            ir.native_imports,
            vec![NativeImportIr {
                module: "math".into(),
                function: "calculate".into()
            }]
        );
    }

    #[test]
    fn rejects_unknown_scriptc_native_module() {
        let source = r#"import { action, wallet, bytes } from "jam"; import { calculate } from "native:missing"; export const run = action({ auth: wallet(), input: { payload: bytes(32) }, execute(ctx, input) { calculate(input.payload); } });"#;
        let error = parse_service_v02(source, "native", "0.2.0", &["math".into()]).unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown or invalid native module"));
    }

    #[test]
    fn keeps_top_level_scriptc_helpers_in_original_source() {
        let source = r#"import { action, wallet, u64 } from "jam";
export function bump(value: number): number { return value + 1; }
export const increment = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) { return bump(input.value); } });"#;
        let ir = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap();
        assert_eq!(ir.source, source);
        assert!(matches!(ir.actions[0].body, ActionBodyIr::ScriptC { .. }));
    }

    #[test]
    fn parses_scriptc_public_action() {
        let source = r#"import { action, publicAction, u64 } from "jam"; export const increment = action({ auth: publicAction(), input: { value: u64 }, execute(ctx, input) { return input.value + 1; } });"#;
        let ir = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap();
        assert_eq!(ir.actions[0].auth, AuthKind::Public);
    }

    #[test]
    fn leaves_determinism_to_scriptc_reachability_policy() {
        let source = r#"import { action, wallet, u64 } from "jam"; export const now = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) { return Date.now(); } });"#;
        let ir = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap();
        assert!(matches!(ir.actions[0].body, ActionBodyIr::ScriptC { .. }));
    }
}
