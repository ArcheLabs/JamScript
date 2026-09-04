// SPDX-License-Identifier: Apache-2.0
#ifndef JAMSCRIPT_JAM_HOST_H
#define JAMSCRIPT_JAM_HOST_H

#include <stdint.h>

#define MINIJAM_HOST_NONE UINT64_MAX

/* These call numbers and the legacy function name are protocol ABI. */
enum minijam_host_call {
  MINIJAM_HOST_GAS = 0,
  MINIJAM_HOST_FETCH = 1,
  MINIJAM_HOST_READ = 3,
  MINIJAM_HOST_WRITE = 4,
  MINIJAM_HOST_NEW = 18,
  MINIJAM_HOST_TRANSFER = 20,
  MINIJAM_HOST_YIELD = 25,
  MINIJAM_HOST_LOG = 100
};

/* Target-owned spelling for new code; values remain the legacy protocol ABI. */
#define JAM_HOST_GAS MINIJAM_HOST_GAS
#define JAM_HOST_FETCH MINIJAM_HOST_FETCH
#define JAM_HOST_READ MINIJAM_HOST_READ
#define JAM_HOST_WRITE MINIJAM_HOST_WRITE
#define JAM_HOST_NEW MINIJAM_HOST_NEW
#define JAM_HOST_TRANSFER MINIJAM_HOST_TRANSFER
#define JAM_HOST_YIELD MINIJAM_HOST_YIELD
#define JAM_HOST_LOG MINIJAM_HOST_LOG
#define JAM_HOST_NONE MINIJAM_HOST_NONE

uint64_t minijam_host_call(uint32_t call, const uint64_t args[6]);

static inline uint64_t minijam_host_call6(uint32_t call, uint64_t a0,
                                          uint64_t a1, uint64_t a2,
                                          uint64_t a3, uint64_t a4,
                                          uint64_t a5) {
  const uint64_t args[6] = {a0, a1, a2, a3, a4, a5};
  return minijam_host_call(call, args);
}

#endif
