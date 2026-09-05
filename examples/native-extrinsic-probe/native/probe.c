#include <minijam/crypto.h>
#include <minijam/minijam.h>
#include <stddef.h>
#include <stdint.h>

#define PROBE_MAX_BYTES (128u * 1024u)

static int equal_bytes(const uint8_t *left, const uint8_t *right, size_t size) {
  uint8_t difference = 0;
  for (size_t index = 0; index < size; index++) difference |= left[index] ^ right[index];
  return difference == 0;
}

uint32_t jamscript_native_probe_verifyProbe_v1(const uint8_t *run_id,
                                                uint32_t run_id_len,
                                                uint64_t *output) {
  static uint8_t replay[PROBE_MAX_BYTES];
  size_t replay_size = 0;
  uint8_t replay_hash[32];
  if (run_id == NULL || output == NULL || run_id_len != 32u) return 1u;
  if (minijam_extrinsic_count() != 1u) return 2u;
  if (minijam_extrinsic(0, replay, sizeof(replay), &replay_size) != MINIJAM_OK ||
      replay_size > sizeof(replay)) return 3u;
  minijam_blake2b_256(replay, replay_size, replay_hash);
  if (!equal_bytes(run_id, replay_hash, 32u)) return 4u;
  *output = replay_size;
  return 0u;
}
