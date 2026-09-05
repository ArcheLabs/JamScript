#include <stddef.h>
#include <stdint.h>

uint32_t jamscript_native_math_calculate_v1(const uint8_t *input,
                                            uint32_t input_len,
                                            uint64_t *output) {
  if (input == NULL || output == NULL || input_len > 256u) return 1u;
  *output = (uint64_t)input_len;
  return 0u;
}
