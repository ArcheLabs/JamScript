use jamscript_ir::{
    ActionBodyIr, ActionIr, AuthKind, ExecuteIr, ExecutionOpIr, FieldIr, NativeImportIr, QueryIr,
    ServiceIr, StateEffectIr, StateIr, StateKeyType, TypeIr, MAX_ACTION_PAYLOAD_BYTES,
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
    parse_service_with_native_modules(source, package_name, package_version, &[])
}

pub fn parse_service_with_native_modules(
    source: &str,
    package_name: &str,
    package_version: &str,
    native_modules: &[String],
) -> Result<ServiceIr, ParseError> {
    parse_service_with_language(source, package_name, package_version, native_modules, "0.1")
}

pub fn parse_service_v02(
    source: &str,
    package_name: &str,
    package_version: &str,
    native_modules: &[String],
) -> Result<ServiceIr, ParseError> {
    reject_scriptc_forbidden_surfaces(source)?;
    parse_service_with_language(source, package_name, package_version, native_modules, "0.2")
}

fn parse_service_with_language(
    source: &str,
    package_name: &str,
    package_version: &str,
    native_modules: &[String],
    language: &str,
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
    for item in module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                collect_import(&import, native_modules, &mut native_imports)?
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
                        Some("action") if language == "0.1" => actions.push(parse_action(
                            binding.id.sym.as_ref(),
                            &init,
                            &native_imports,
                        )?),
                        Some("action") => {
                            actions.push(parse_scriptc_action(binding.id.sym.as_ref(), &init)?)
                        }
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
                    states.push(parse_state(binding.id.sym.as_ref(), &init)?);
                }
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
    if actions.len() != 1 {
        return Err(diag(
            "1006",
            "the v0.1 vertical slice requires exactly one exported action",
        ));
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
        if !matches!(
            name.as_str(),
            "action"
                | "wallet"
                | "publicAction"
                | "u64"
                | "bytes"
                | "address"
                | "stateMap"
                | "query"
        ) {
            return Err(diag(
                "1009",
                format!("`{name}` is not part of the v0.1 standard library"),
            ));
        }
    }
    Ok(())
}

fn parse_action(
    name: &str,
    init: &Expr,
    native_imports: &[NativeImportIr],
) -> Result<ActionIr, ParseError> {
    let config = call_object(init, "action", "1011")?;
    let mut auth = None;
    let mut input = None;
    let mut execute = None;
    for prop in &config.props {
        let PropOrSpread::Prop(prop) = prop else {
            return Err(diag("1013", "spread properties are not supported"));
        };
        match &**prop {
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("auth") => {
                auth = Some(parse_auth(&kv.value)?)
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("input") => {
                input = Some(parse_input(&kv.value)?)
            }
            Prop::Method(method) if key_name(&method.key).as_deref() == Some("execute") => {
                execute = Some(parse_execute(&method.function.body, native_imports)?);
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("execute") => {
                let Expr::Fn(function) = &*kv.value else {
                    return Err(diag("1014", "execute must be a function"));
                };
                execute = Some(parse_execute(&function.function.body, native_imports)?);
            }
            Prop::KeyValue(kv) => {
                return Err(diag(
                    "1015",
                    format!(
                        "unsupported action property `{}`",
                        key_name(&kv.key).unwrap_or_default()
                    ),
                ))
            }
            Prop::Method(method) => {
                return Err(diag(
                    "1015",
                    format!(
                        "unsupported action property `{}`",
                        key_name(&method.key).unwrap_or_default()
                    ),
                ))
            }
            _ => return Err(diag("1013", "getters and setters are not supported")),
        }
    }
    let input = input.ok_or_else(|| diag("1017", "action must declare an input object"))?;
    validate_input(&input)?;
    let body = execute.ok_or_else(|| diag("1018", "action must declare execute(ctx, input)"))?;
    Ok(ActionIr {
        name: name.into(),
        auth: auth.ok_or_else(|| {
            diag(
                "1016",
                "action must declare auth: wallet() or publicAction()",
            )
        })?,
        input,
        body,
    })
}

fn parse_scriptc_action(name: &str, init: &Expr) -> Result<ActionIr, ParseError> {
    let config = call_object(init, "action", "1110")?;
    let mut auth = None;
    let mut input = None;
    let mut script_source = None;
    for prop in &config.props {
        let PropOrSpread::Prop(prop) = prop else {
            return Err(diag("1111", "spread properties are not supported"));
        };
        match &**prop {
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("auth") => {
                auth = Some(parse_auth(&kv.value)?);
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("input") => {
                input = Some(parse_input(&kv.value)?);
            }
            Prop::Method(method) if key_name(&method.key).as_deref() == Some("execute") => {
                script_source = Some(scriptc_source_from_body(name, &method.function.body)?);
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("execute") => {
                let Expr::Fn(function) = &*kv.value else {
                    return Err(diag("1112", "execute must be a function"));
                };
                script_source = Some(scriptc_source_from_body(name, &function.function.body)?);
            }
            _ => return Err(diag("1113", "unsupported action property for ScriptC")),
        }
    }
    let input = input.ok_or_else(|| diag("1114", "action must declare an input object"))?;
    validate_input(&input)?;
    if input.len() != 1 || input[0].ty != TypeIr::U64 {
        return Err(diag(
            "1119",
            "ScriptC M1 compute requires exactly one u64 input field",
        ));
    }
    let source =
        script_source.ok_or_else(|| diag("1115", "action must declare execute(ctx, input)"))?;
    Ok(ActionIr {
        name: name.into(),
        auth: auth.ok_or_else(|| diag("1116", "action must declare auth"))?,
        input,
        body: ActionBodyIr::ScriptC {
            symbol: name.into(),
            source,
            state_effect: None,
        },
    })
}

fn scriptc_source_from_body(name: &str, body: &Option<BlockStmt>) -> Result<String, ParseError> {
    let operation = parse_execution_operation(body, &[])?;
    let expression = match operation {
        ExecutionOpIr::ReturnInputField { .. } => "input".to_owned(),
        ExecutionOpIr::AddInputField { value, .. } => format!("input + {value}"),
        ExecutionOpIr::ReturnInteger { value } => value.to_string(),
        ExecutionOpIr::NativeBytesToU64 { .. } => {
            return Err(diag(
                "1118",
                "ScriptC M1 action does not support native compute yet",
            ));
        }
    };
    Ok(format!(
        "export function {name}(input: number): number {{ return {expression}; }}\n"
    ))
}

fn reject_scriptc_forbidden_surfaces(source: &str) -> Result<(), ParseError> {
    const FORBIDDEN: &[(&str, &str)] = &[
        ("Date", "Date"),
        ("Date.now", "Date.now"),
        ("Math.random", "Math.random"),
        ("process", "process"),
        ("process.env", "process.env"),
        ("fs", "fs"),
        ("fetch", "fetch"),
        ("setTimeout", "timers"),
        ("setInterval", "timers"),
        ("Promise", "async/Promise"),
        ("async", "async/Promise"),
        ("eval", "eval"),
        ("Function", "Function constructor"),
        ("require", "dynamic module loading"),
        ("globalThis", "global object"),
        ("import(", "dynamic module loading"),
    ];
    for (needle, label) in FORBIDDEN {
        let token_match = source
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|token| token == *needle);
        if token_match || source.contains(needle) {
            return Err(diag(
                "1117",
                format!("ScriptC deterministic profile forbids reachable surface `{label}`"),
            ));
        }
    }
    Ok(())
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

fn parse_input(expr: &Expr) -> Result<Vec<FieldIr>, ParseError> {
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
                ty: parse_type(&kv.value)?,
            })
        })
        .collect()
}

fn parse_type(expr: &Expr) -> Result<TypeIr, ParseError> {
    if let Expr::Ident(name) = expr {
        return match name.sym.as_ref() {
            "u64" => Ok(TypeIr::U64),
            "address" => Ok(TypeIr::Address),
            other => Err(diag(
                "1023",
                format!("unsupported v0.1 input type `{other}`"),
            )),
        };
    }
    let Expr::Call(call) = expr else {
        return Err(diag("1023", "type must be u64, address, or bytes(N)"));
    };
    let Callee::Expr(callee) = &call.callee else {
        return Err(diag("1023", "type must be u64, address, or bytes(N)"));
    };
    let Expr::Ident(name) = &**callee else {
        return Err(diag("1023", "type must be u64, address, or bytes(N)"));
    };
    if name.sym != *"bytes" || call.args.len() != 1 {
        return Err(diag("1023", "type must be u64, address, or bytes(N)"));
    }
    let max = integer_literal(&call.args[0].expr)?
        .ok_or_else(|| diag("1023", "bytes(N) requires an integer bound"))?;
    if !(1..=MAX_ACTION_PAYLOAD_BYTES as u128).contains(&max) {
        return Err(diag("1036", "bytes(N) must satisfy 1 <= N <= 1000000"));
    }
    Ok(TypeIr::Bytes { max: max as u32 })
}

fn parse_execute(
    body: &Option<BlockStmt>,
    native_imports: &[NativeImportIr],
) -> Result<ActionBodyIr, ParseError> {
    let Some(body) = body else {
        return Err(diag("1024", "execute must have a function body"));
    };
    if body.stmts.len() == 1 {
        return Ok(ActionBodyIr::Execute(ExecuteIr {
            operation: parse_execution_operation(&Some(body.clone()), native_imports)?,
            state_effect: None,
        }));
    }
    if body.stmts.len() != 2 {
        return Err(diag(
            "1025",
            "execute must contain a computation and at most one state operation",
        ));
    }
    let Stmt::Decl(Decl::Var(declaration)) = &body.stmts[0] else {
        return Err(diag(
            "1025",
            "execute must bind its result before state access",
        ));
    };
    if declaration.decls.len() != 1 {
        return Err(diag("1025", "execute must bind exactly one result"));
    }
    let Pat::Ident(binding) = &declaration.decls[0].name else {
        return Err(diag("1025", "execute result binding must be an identifier"));
    };
    if binding.id.sym != *"score" {
        return Err(diag("1025", "execute result must be named score"));
    }
    let Some(initializer) = &declaration.decls[0].init else {
        return Err(diag("1025", "execute result must have an initializer"));
    };
    let operation = parse_native_expression(initializer, native_imports)?;
    let Stmt::Expr(ExprStmt { expr, .. }) = &body.stmts[1] else {
        return Err(diag(
            "1038",
            "execute state operation must be an expression",
        ));
    };
    Ok(ActionBodyIr::Execute(ExecuteIr {
        operation,
        state_effect: Some(parse_commit_expression(expr)?),
    }))
}

fn parse_native_expression(
    expr: &Expr,
    native_imports: &[NativeImportIr],
) -> Result<ExecutionOpIr, ParseError> {
    let Expr::Call(call) = expr else {
        return Err(diag(
            "1037",
            "execute computation must call a native function",
        ));
    };
    let Callee::Expr(callee) = &call.callee else {
        return Err(diag("1028", "unsupported execute expression"));
    };
    let Expr::Ident(function) = &**callee else {
        return Err(diag("1028", "unsupported execute expression"));
    };
    let Some(native) = native_imports
        .iter()
        .find(|item| function.sym == item.function)
    else {
        return Err(diag(
            "1037",
            "execute must call an imported native function",
        ));
    };
    if call.args.len() != 1 {
        return Err(diag(
            "1037",
            "native replay accepts exactly one input field",
        ));
    }
    let Expr::Member(member) = &*call.args[0].expr else {
        return Err(diag("1037", "native replay argument must be input.field"));
    };
    Ok(ExecutionOpIr::NativeBytesToU64 {
        module: native.module.clone(),
        function: native.function.clone(),
        field: input_member(member)?,
    })
}

fn parse_execution_operation(
    body: &Option<BlockStmt>,
    native_imports: &[NativeImportIr],
) -> Result<ExecutionOpIr, ParseError> {
    let Some(body) = body else {
        return Err(diag("1024", "compute must have a function body"));
    };
    if body.stmts.len() != 1 {
        return Err(diag(
            "1025",
            "compute must contain exactly one return statement",
        ));
    }
    let Stmt::Return(ReturnStmt {
        arg: Some(expr), ..
    }) = &body.stmts[0]
    else {
        return Err(diag(
            "1025",
            "compute must contain exactly one return statement",
        ));
    };
    match &**expr {
        Expr::Call(call) => parse_native_expression(&Expr::Call(call.clone()), native_imports),
        Expr::Member(member) => Ok(ExecutionOpIr::ReturnInputField {
            field: input_member(member)?,
        }),
        Expr::Bin(binary) if binary.op == BinaryOp::Add => {
            let Expr::Member(member) = &*binary.left else {
                return Err(diag("1027", "arithmetic must be `input.field + integer`"));
            };
            Ok(ExecutionOpIr::AddInputField {
                field: input_member(member)?,
                value: integer_literal(&binary.right)?
                    .ok_or_else(|| diag("1027", "arithmetic requires an integer literal"))?,
            })
        }
        Expr::Lit(Lit::Num(_)) => Ok(ExecutionOpIr::ReturnInteger {
            value: integer_literal(expr)?
                .ok_or_else(|| diag("1028", "unsupported numeric literal"))?,
        }),
        _ => Err(diag("1028", "unsupported compute expression")),
    }
}

fn parse_commit_expression(expr: &Expr) -> Result<StateEffectIr, ParseError> {
    let Expr::Call(call) = expr else {
        return Err(diag("1038", "execute must call state.set or state.max"));
    };
    let Callee::Expr(callee) = &call.callee else {
        return Err(diag("1038", "execute must call state.set or state.max"));
    };
    let Expr::Member(member) = &**callee else {
        return Err(diag("1038", "execute must call state.set or state.max"));
    };
    let Expr::Ident(state) = &*member.obj else {
        return Err(diag("1038", "execute must call state.set or state.max"));
    };
    let MemberProp::Ident(operation) = &member.prop else {
        return Err(diag("1038", "commit operation must be set or max"));
    };
    if call.args.len() != 2 {
        return Err(diag("1038", "execute state operation takes key and result"));
    }
    let Expr::Member(key) = &*call.args[0].expr else {
        return Err(diag("1039", "execute key must be ctx.sender"));
    };
    let Expr::Ident(ctx) = &*key.obj else {
        return Err(diag("1039", "execute key must be ctx.sender"));
    };
    let MemberProp::Ident(sender) = &key.prop else {
        return Err(diag("1039", "execute key must be ctx.sender"));
    };
    if ctx.sym != *"ctx" || sender.sym != *"sender" {
        return Err(diag("1039", "execute key must be ctx.sender"));
    }
    let Expr::Ident(value) = &*call.args[1].expr else {
        return Err(diag("1040", "execute value must be the score result"));
    };
    if value.sym != *"score" {
        return Err(diag("1040", "execute value must be the result named score"));
    }
    match operation.sym.as_ref() {
        "set" => Ok(StateEffectIr::Set {
            state: state.sym.to_string(),
        }),
        "max" => Ok(StateEffectIr::Max {
            state: state.sym.to_string(),
        }),
        _ => Err(diag("1038", "execute operation must be set or max")),
    }
}

fn parse_state(name: &str, init: &Expr) -> Result<StateIr, ParseError> {
    let object = call_object(init, "stateMap", "1041")?;
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
            Some("key") => key_type = Some(parse_type(&kv.value)?),
            Some("value") => value_type = Some(parse_type(&kv.value)?),
            _ => {
                return Err(diag(
                    "1043",
                    "stateMap supports schema, key, and value only",
                ))
            }
        }
    }
    let schema = schema.ok_or_else(|| diag("1044", "stateMap requires schema"))?;
    if schema.is_empty() || schema.len() > 64 || !schema.is_ascii() {
        return Err(diag(
            "1045",
            "state schema must be non-empty ASCII of at most 64 bytes",
        ));
    }
    let key_type = key_type.ok_or_else(|| diag("1046", "stateMap requires key"))?;
    let value_type = value_type.ok_or_else(|| diag("1047", "stateMap requires value"))?;
    if key_type != TypeIr::Address || value_type != TypeIr::U64 {
        return Err(diag(
            "1048",
            "M4 stateMap only supports address keys and u64 values",
        ));
    }
    Ok(StateIr {
        name: name.into(),
        schema,
        key_type: StateKeyType::Address,
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
        .try_fold(0u32, |total, field| {
            let size = match field.ty {
                TypeIr::U64 => 8,
                TypeIr::Bytes { max } => max.checked_add(4).ok_or(())?,
                _ => return Err(()),
            };
            total.checked_add(size).ok_or(())
        })
        .map_err(|_| diag("1056", "action input exceeds the bounded payload limit"))?;
    if total > MAX_ACTION_PAYLOAD_BYTES {
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
fn input_member(member: &MemberExpr) -> Result<String, ParseError> {
    let Expr::Ident(object) = &*member.obj else {
        return Err(diag("1026", "compute may only read input.field"));
    };
    if object.sym != *"input" {
        return Err(diag("1026", "compute may only read input.field"));
    }
    let MemberProp::Ident(property) = &member.prop else {
        return Err(diag("1026", "computed input properties are not supported"));
    };
    Ok(property.sym.to_string())
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
    fn parses_game_vertical_slice() {
        let source = r#"import { action, wallet, bytes, address, stateMap, query, u64 } from "jam"; import { replay } from "native:game"; const bestScore = stateMap({ schema: "best-score/v1", key: address, value: u64 }); export const submitRun = action({ auth: wallet(), input: { run: bytes(64) }, execute(ctx, input) { const score = replay(input.run); bestScore.max(ctx.sender, score); } }); export const getBestScore = query(bestScore);"#;
        let ir =
            parse_service_with_native_modules(source, "game", "0.1.0", &["game".into()]).unwrap();
        assert_eq!(ir.actions[0].input[0].ty, TypeIr::Bytes { max: 64 });
        assert_eq!(ir.queries[0].state, "bestScore");
        assert!(matches!(
            ir.actions[0].body,
            ActionBodyIr::Execute(ExecuteIr {
                operation: ExecutionOpIr::NativeBytesToU64 { .. },
                ..
            })
        ));
    }

    #[test]
    fn parses_execute_as_one_application_body() {
        let source = r#"import { action, wallet, bytes, address, stateMap, query, u64 } from "jam"; import { replay } from "native:game"; const bestScore = stateMap({ schema: "best-score/v1", key: address, value: u64 }); export const submitRun = action({ auth: wallet(), input: { run: bytes(64) }, execute(ctx, input) { const score = replay(input.run); bestScore.max(ctx.sender, score); } }); export const getBestScore = query(bestScore);"#;
        let ir =
            parse_service_with_native_modules(source, "game", "0.1.0", &["game".into()]).unwrap();
        assert!(matches!(
            ir.actions[0].body,
            ActionBodyIr::Execute(ExecuteIr {
                operation: ExecutionOpIr::NativeBytesToU64 { .. },
                state_effect: Some(StateEffectIr::Max { .. }),
            })
        ));
    }
    #[test]
    fn rejects_unbounded_bytes() {
        let source = r#"import { action, wallet, bytes } from "jam"; export const add = action({ auth: wallet(), input: { value: bytes(0) }, execute(ctx, input) { return input.value; } });"#;
        assert!(parse_service(source, "x", "0.1.0").is_err());
    }

    #[test]
    fn parses_scriptc_wallet_action() {
        let source = r#"import { action, wallet, u64 } from "jam"; export const increment = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) { return input.value + 1; } });"#;
        let ir = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap();
        assert_eq!(ir.actions[0].auth, AuthKind::Wallet);
        assert!(
            matches!(ir.actions[0].body, ActionBodyIr::ScriptC { ref symbol, .. } if symbol == "increment")
        );
    }

    #[test]
    fn parses_scriptc_public_action() {
        let source = r#"import { action, publicAction, u64 } from "jam"; export const increment = action({ auth: publicAction(), input: { value: u64 }, execute(ctx, input) { return input.value + 1; } });"#;
        let ir = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap();
        assert_eq!(ir.actions[0].auth, AuthKind::Public);
    }

    #[test]
    fn rejects_scriptc_nondeterministic_surface() {
        let source = r#"import { action, wallet, u64 } from "jam"; export const now = action({ auth: wallet(), input: { value: u64 }, execute(ctx, input) { return Date.now(); } });"#;
        let error = parse_service_v02(source, "counter", "0.2.0", &[]).unwrap_err();
        assert!(error.to_string().contains("JAM1117"));
    }
}
