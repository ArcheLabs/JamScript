use crate::{ArtifactBuildContext, ManagementPolicyConfig};
use jamscript_ir::{action_selector, AuthKind, ServiceIr};

/// Generate the C object that binds a ScriptC application to the precompiled
/// guest runtime. This is an internal ABI and is deliberately independent of
/// the public service ABI JSON.
pub fn generate_service_descriptor_c(
    ir: &ServiceIr,
    context: ArtifactBuildContext,
) -> Result<String, String> {
    if ir.actions.is_empty() {
        return Err("service descriptor requires at least one action".into());
    }

    let mut source = String::from(
        "#include <stdint.h>\n#include <stddef.h>\n#include \"jam/service-descriptor-v1.h\"\n\n",
    );
    source.push_str("extern void jamscript_scriptc_service_init(void);\n");
    for action in &ir.actions {
        source.push_str(&format!(
            "extern void jamscript_scriptc_{}_entry_v1(const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t **, size_t *);\n",
            action.name
        ));
    }
    source.push('\n');

    for (index, state) in ir.states.iter().enumerate() {
        source.push_str(&format!(
            "static const uint8_t jamscript_namespace_{index}[] = {{{}}};\n",
            state
                .schema
                .as_bytes()
                .iter()
                .map(|byte| byte.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !ir.states.is_empty() {
        source
            .push_str("\nstatic const JamScriptNamespaceDescriptorV1 jamscript_namespaces[] = {\n");
        for (index, state) in ir.states.iter().enumerate() {
            source.push_str(&format!(
                "    {{ jamscript_namespace_{index}, {} }},\n",
                state.schema.len()
            ));
        }
        source.push_str("};\n\n");
    }

    source.push_str("static const JamScriptActionDescriptorV1 jamscript_actions[] = {\n");
    for action in &ir.actions {
        let selector = action_selector(&action.name)
            .iter()
            .map(|byte| format!("0x{byte:02x}"))
            .collect::<Vec<_>>()
            .join(", ");
        let auth = match action.auth {
            AuthKind::Public => "JAMSCRIPT_AUTH_PUBLIC_V1",
            AuthKind::Wallet => "JAMSCRIPT_AUTH_WALLET_V1",
            AuthKind::Ownership => "JAMSCRIPT_AUTH_OWNERSHIP_V1",
        };
        source.push_str(&format!(
            "    {{ {{ {selector} }}, {auth}, {{ 0 }}, jamscript_scriptc_{}_entry_v1 }},\n",
            action.name
        ));
    }
    source.push_str("};\n\n");

    let service_key = bytes_literal(&context.service_key);
    let service_instance_id = bytes_literal(&context.service_instance_id);
    let (management_kind, management_account) = match context.management_policy {
        ManagementPolicyConfig::Immutable => (0u8, [0u8; 32]),
        ManagementPolicyConfig::Key { account } => (1u8, account),
    };
    let namespace_count = ir.states.len();
    source.push_str(&format!(
        concat!(
            "const JamScriptServiceDescriptorV1 jamscript_service_descriptor_v1 = {{\n",
            "    1,\n",
            "    {},\n",
            "    jamscript_actions,\n",
            "    {},\n",
            "    {},\n",
            "    {{ {} }},\n",
            "    {{ {} }},\n",
            "    {},\n",
            "    {{ 0 }},\n",
            "    {{ {} }},\n",
            "    jamscript_scriptc_service_init\n",
            "}};\n"
        ),
        ir.actions.len(),
        namespace_count,
        if namespace_count == 0 {
            "(const JamScriptNamespaceDescriptorV1 *)0"
        } else {
            "jamscript_namespaces"
        },
        service_key,
        service_instance_id,
        management_kind,
        management_account
            .iter()
            .map(|byte| byte.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok(source)
}

fn bytes_literal(bytes: &[u8; 32]) -> String {
    bytes
        .iter()
        .map(|byte| byte.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
