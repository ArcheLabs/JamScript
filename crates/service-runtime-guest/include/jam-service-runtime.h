#ifndef JAM_SERVICE_RUNTIME_H
#define JAM_SERVICE_RUNTIME_H

#include <stdint.h>

/* Stable C ABI sketch for the managed-state application surface. */
uint32_t jam_state_get(
    const uint8_t *key,
    uint32_t key_len,
    uint8_t *output,
    uint32_t capacity,
    uint32_t *output_len);

uint32_t jam_state_set(
    const uint8_t *key,
    uint32_t key_len,
    const uint8_t *value,
    uint32_t value_len);

uint32_t jam_state_delete(
    const uint8_t *key,
    uint32_t key_len);

#endif
