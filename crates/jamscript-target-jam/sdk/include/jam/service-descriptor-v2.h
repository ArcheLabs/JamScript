#ifndef JAMSCRIPT_SERVICE_DESCRIPTOR_V2_H
#define JAMSCRIPT_SERVICE_DESCRIPTOR_V2_H

#include <stddef.h>
#include <stdint.h>

#define JAMSCRIPT_SERVICE_DESCRIPTOR_V2 2u
#define JAMSCRIPT_AUTH_PUBLIC_V1 0u
#define JAMSCRIPT_AUTH_WALLET_V1 1u
#define JAMSCRIPT_AUTH_OWNERSHIP_V1 2u

typedef void (*JamScriptActionEntryV1)(
    const uint8_t *payload,
    size_t payload_len,
    const uint8_t *auth_context,
    size_t auth_context_len,
    const uint8_t *state_view,
    size_t state_view_len,
    const uint8_t **output,
    size_t *output_len);

typedef struct {
    uint8_t selector[8];
    uint8_t auth_kind;
    uint8_t reserved[7];
    JamScriptActionEntryV1 entry;
} JamScriptActionDescriptorV1;

typedef struct {
    const uint8_t *bytes;
    size_t len;
} JamScriptNamespaceDescriptorV1;

typedef void (*JamScriptServiceInitV1)(void);

typedef struct {
    uint32_t version;
    uint32_t action_count;
    const JamScriptActionDescriptorV1 *actions;
    uint32_t namespace_count;
    const JamScriptNamespaceDescriptorV1 *namespaces;
    uint8_t service_key[32];
    uint8_t service_instance_id[32];
    uint8_t management_policy_kind;
    uint8_t reserved[7];
    uint8_t management_account[32];
    JamScriptServiceInitV1 init;
    uint32_t heap_initial_bytes;
    uint32_t heap_max_bytes;
} JamScriptServiceDescriptorV2;

extern const JamScriptServiceDescriptorV2 jamscript_service_descriptor_v2;

#endif
