// SPDX-License-Identifier: Apache-2.0
#ifndef JAMSCRIPT_JAM_CRYPTO_H
#define JAMSCRIPT_JAM_CRYPTO_H

#include <stddef.h>
#include <stdint.h>

/* Legacy symbol retained for ABI compatibility with existing services. */
void minijam_blake2b_256(const void *input, size_t input_size,
                         uint8_t output[32]);

#endif
