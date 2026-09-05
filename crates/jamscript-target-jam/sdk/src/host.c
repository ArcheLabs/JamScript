// SPDX-License-Identifier: Apache-2.0
#include <jam/host.h>

#ifndef JAM_HOST_TEST
#if defined(__riscv)
struct __attribute__((packed)) jam_extern_metadata_v2 {
  uint8_t version;
  uint32_t flags;
  uint32_t symbol_length;
  const uint8_t *symbol;
  uint8_t input_regs;
  uint8_t output_regs;
  uint8_t has_index;
  uint32_t index;
};

#define JAM_IMPORT_METADATA(NAME, ID)                                      \
  static const uint8_t NAME##_symbol[]                                    \
      __attribute__((section(".polkavm_metadata"), used)) = #NAME;       \
  static const struct jam_extern_metadata_v2 NAME##_metadata              \
      __attribute__((section(".polkavm_metadata"), used)) = {             \
          2, 0, sizeof(NAME##_symbol) - 1, NAME##_symbol, 6, 2, 1, ID}

JAM_IMPORT_METADATA(minijam_gas, JAM_HOST_GAS);
JAM_IMPORT_METADATA(minijam_fetch, JAM_HOST_FETCH);
JAM_IMPORT_METADATA(minijam_read, JAM_HOST_READ);
JAM_IMPORT_METADATA(minijam_write, JAM_HOST_WRITE);
JAM_IMPORT_METADATA(minijam_new, JAM_HOST_NEW);
JAM_IMPORT_METADATA(minijam_transfer, JAM_HOST_TRANSFER);
JAM_IMPORT_METADATA(minijam_yield, JAM_HOST_YIELD);
JAM_IMPORT_METADATA(minijam_log, JAM_HOST_LOG);
#undef JAM_IMPORT_METADATA
#endif

uint64_t minijam_host_call(uint32_t call, const uint64_t args[6]) {
#if defined(__riscv)
  register uint64_t r0 __asm__("a0") = args[0];
  register uint64_t r1 __asm__("a1") = args[1];
  register uint64_t r2 __asm__("a2") = args[2];
  register uint64_t r3 __asm__("a3") = args[3];
  register uint64_t r4 __asm__("a4") = args[4];
  register uint64_t r5 __asm__("a5") = args[5];
#define JAM_ECALLI(METADATA)                                                \
  __asm__ volatile(".insn r 0xb, 0, 0, zero, zero, zero\n"                  \
                   ".8byte %c6\n"                                        \
                   : "+r"(r0)                                              \
                   : "r"(r1), "r"(r2), "r"(r3), "r"(r4), "r"(r5),          \
                     "i"(&(METADATA))                                      \
                   : "memory")
  switch (call) {
    case JAM_HOST_GAS: JAM_ECALLI(minijam_gas_metadata); break;
    case JAM_HOST_FETCH: JAM_ECALLI(minijam_fetch_metadata); break;
    case JAM_HOST_READ: JAM_ECALLI(minijam_read_metadata); break;
    case JAM_HOST_WRITE: JAM_ECALLI(minijam_write_metadata); break;
    case JAM_HOST_NEW: JAM_ECALLI(minijam_new_metadata); break;
    case JAM_HOST_TRANSFER: JAM_ECALLI(minijam_transfer_metadata); break;
    case JAM_HOST_YIELD: JAM_ECALLI(minijam_yield_metadata); break;
    case JAM_HOST_LOG: JAM_ECALLI(minijam_log_metadata); break;
    default: return JAM_HOST_NONE;
  }
#undef JAM_ECALLI
  return r0;
#else
#error "Jam target SDK must be compiled for the pinned PolkaVM target"
#endif
}
#endif
