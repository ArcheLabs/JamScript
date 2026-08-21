use jamscript_ir::{ActionIr, AuthKind, ComputeIr, FieldIr, ServiceIr, TypeIr};
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
    for item in module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => validate_import(&import)?,
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
                        return Err(diag("1005", "exported action must have an initializer"));
                    };
                    actions.push(parse_action(&binding.id.sym.to_string(), &init)?);
                }
            }
            ModuleItem::Stmt(Stmt::Empty(..)) => {}
            _ => {
                return Err(diag(
                    "1001",
                    "only imports from `jam` and exported actions are supported in v0.1",
                ))
            }
        }
    }
    if actions.len() != 1 {
        return Err(diag(
            "1006",
            "the M0 vertical slice requires exactly one exported action",
        ));
    }
    Ok(ServiceIr {
        package_name: package_name.into(),
        package_version: package_version.into(),
        actions,
    })
}

fn validate_import(import: &ImportDecl) -> Result<(), ParseError> {
    if import.src.value != "jam" {
        return Err(diag(
            "1007",
            "Service imports must come from the `jam` standard library",
        ));
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
                | "Address"
                | "Bytes"
                | "String"
                | "u32"
                | "u64"
                | "u128"
                | "bool"
        ) {
            return Err(diag(
                "1009",
                format!("`{name}` is not part of the v0.1 standard library"),
            ));
        }
    }
    Ok(())
}

fn parse_action(name: &str, init: &Expr) -> Result<ActionIr, ParseError> {
    let Expr::Call(call) = init else {
        return Err(diag(
            "1010",
            format!("`{name}` must be initialized with action({{...}})"),
        ));
    };
    let Callee::Expr(callee) = &call.callee else {
        return Err(diag("1011", "only the `action` helper is supported"));
    };
    let Expr::Ident(callee) = &**callee else {
        return Err(diag("1011", "only the `action` helper is supported"));
    };
    if callee.sym != *"action" || call.args.len() != 1 {
        return Err(diag(
            "1011",
            "only action({...}) with one configuration object is supported",
        ));
    }
    let Expr::Object(config) = &*call.args[0].expr else {
        return Err(diag(
            "1012",
            "action configuration must be an object literal",
        ));
    };
    let mut auth = None;
    let mut input = None;
    let mut compute = None;
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
            Prop::Method(method) if key_name(&method.key).as_deref() == Some("compute") => {
                compute = Some(parse_compute(&method.function.body)?)
            }
            Prop::KeyValue(kv) if key_name(&kv.key).as_deref() == Some("compute") => {
                let Expr::Fn(function) = &*kv.value else {
                    return Err(diag("1014", "compute must be a function"));
                };
                compute = Some(parse_compute(&function.function.body)?);
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
    Ok(ActionIr {
        name: name.into(),
        auth: auth.ok_or_else(|| {
            diag(
                "1016",
                "action must declare auth: wallet() or publicAction()",
            )
        })?,
        input: input.ok_or_else(|| diag("1017", "action must declare an input object"))?,
        compute: compute.ok_or_else(|| {
            diag(
                "1018",
                "action must declare compute(ctx, input) in the M0 slice",
            )
        })?,
    })
}

fn parse_auth(expr: &Expr) -> Result<AuthKind, ParseError> {
    let Expr::Call(call) = expr else {
        return Err(diag("1019", "auth must be wallet() or publicAction()"));
    };
    let Callee::Expr(callee) = &call.callee else {
        return Err(diag("1019", "auth must be wallet() or publicAction()"));
    };
    let Expr::Ident(name) = &**callee else {
        return Err(diag("1019", "auth must be wallet() or publicAction()"));
    };
    match name.sym.as_ref() {
        "wallet" if call.args.is_empty() => Ok(AuthKind::Wallet),
        "publicAction" if call.args.is_empty() => Ok(AuthKind::Public),
        _ => Err(diag("1019", "auth must be wallet() or publicAction()")),
    }
}

fn parse_input(expr: &Expr) -> Result<Vec<FieldIr>, ParseError> {
    let Expr::Object(object) = expr else {
        return Err(diag(
            "1020",
            "input must be an object schema in the M0 slice",
        ));
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
    let Expr::Ident(name) = expr else {
        return Err(diag(
            "1023",
            "only fixed-width primitive types are supported in the M0 slice",
        ));
    };
    match name.sym.as_ref() {
        "u64" => Ok(TypeIr::U64),
        "u32" => Ok(TypeIr::U32),
        "u128" => Ok(TypeIr::U128),
        "bool" => Ok(TypeIr::Bool),
        other => Err(diag("1023", format!("unsupported input type `{other}`"))),
    }
}

fn parse_compute(body: &Option<BlockStmt>) -> Result<ComputeIr, ParseError> {
    let Some(body) = body else {
        return Err(diag("1024", "compute must have a function body"));
    };
    if body.stmts.len() != 1 {
        return Err(diag(
            "1025",
            "compute must contain exactly one return statement in the M0 slice",
        ));
    }
    let Stmt::Return(ReturnStmt {
        arg: Some(expr), ..
    }) = &body.stmts[0]
    else {
        return Err(diag(
            "1025",
            "compute must contain exactly one return statement in the M0 slice",
        ));
    };
    match &**expr {
        Expr::Member(member) => { let Expr::Ident(object) = &*member.obj else { return Err(diag("1026", "compute may only read `input.field`")); }; if object.sym != *"input" { return Err(diag("1026", "compute may only read `input.field`")); } let MemberProp::Ident(property) = &member.prop else { return Err(diag("1026", "computed input properties are not supported")); }; Ok(ComputeIr::ReturnInputField { field: property.sym.to_string() }) }
        Expr::Bin(binary) if binary.op == BinaryOp::Add => { let Expr::Member(member) = &*binary.left else { return Err(diag("1027", "M0 arithmetic must be `input.field + integer`")); }; let Expr::Ident(object) = &*member.obj else { return Err(diag("1027", "M0 arithmetic must use input.field")); }; if object.sym != *"input" { return Err(diag("1027", "M0 arithmetic must use input.field")); }; let MemberProp::Ident(property) = &member.prop else { return Err(diag("1027", "computed input properties are not supported")); }; let value = integer_literal(&binary.right).ok_or_else(|| diag("1027", "M0 arithmetic requires an integer literal on the right"))?; Ok(ComputeIr::AddInputField { field: property.sym.to_string(), value }) }
        Expr::Lit(Lit::Num(value)) if value.value.fract() == 0.0 && value.value >= 0.0 => {
            Ok(ComputeIr::ReturnInteger { value: value.value as u128 })
        }
        _ => Err(diag("1028", "unsupported compute expression; use `input.field`, `input.field + integer`, or an integer literal")),
    }
}

fn integer_literal(expr: &Expr) -> Option<u128> {
    match expr {
        Expr::Lit(Lit::Num(value)) if value.value.fract() == 0.0 && value.value >= 0.0 => {
            Some(value.value as u128)
        }
        _ => None,
    }
}
fn key_name(key: &PropName) -> Option<String> {
    match key {
        PropName::Ident(ident) => Some(ident.sym.to_string()),
        _ => None,
    }
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
    fn parses_minimal_action() {
        let ir = parse_service(r#"import { action, wallet, u64 } from "jam"; export const add = action({ auth: wallet(), input: { value: u64 }, compute(ctx, input) { return input.value + 1; } });"#, "counter", "0.1.0").unwrap();
        assert_eq!(ir.actions[0].name, "add");
        assert_eq!(
            ir.actions[0].compute,
            ComputeIr::AddInputField {
                field: "value".into(),
                value: 1
            }
        );
    }
}
