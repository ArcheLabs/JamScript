// SPDX-License-Identifier: Apache-2.0
#include <assert.h>
#include <stdint.h>
#include <stddef.h>

#include <jam/abi.h>
#include <jam/host.h>

static uint64_t last_read_args[6];
static uint64_t mock_read_result;

uint64_t minijam_host_call(uint32_t call, const uint64_t args[6]) {
  assert(call == MINIJAM_HOST_READ);
  for (size_t index = 0; index < 6; ++index) last_read_args[index] = args[index];
  return mock_read_result;
}

int main(void) {
  _Static_assert(UINT32_MAX != MINIJAM_HOST_NONE,
                 "the explicit u32 service-id space must not contain HOST_NONE");

  const uint8_t key[] = {0x01};
  uint8_t output[8] = {0};
  size_t output_size = 0;

  mock_read_result = MINIJAM_HOST_NONE;
  assert(minijam_storage_read(key, sizeof(key), output, sizeof(output),
                              &output_size) == MINIJAM_NOT_FOUND);
  assert(last_read_args[0] == MINIJAM_HOST_NONE);

  const uint32_t service_id = 0x12345678u;
  mock_read_result = 3;
  assert(minijam_service_storage_read(service_id, key, sizeof(key), output,
                                      sizeof(output), &output_size) ==
         MINIJAM_OK);
  assert(last_read_args[0] == (uint64_t)service_id);
  assert(last_read_args[0] != MINIJAM_HOST_NONE);
  assert(output_size == 3);

  return 0;
}
