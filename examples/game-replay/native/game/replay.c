#include "replay.h"

enum {
  GAME_ENGINE_VERSION = 1u,
  OP_MOVE = 1u,
  OP_ATTACK = 2u,
  OP_BONUS = 3u,
  MAX_STEPS = 64u,
};

static uint32_t read_u32(const uint8_t *input) {
  return ((uint32_t)input[0]) | ((uint32_t)input[1] << 8) |
         ((uint32_t)input[2] << 16) | ((uint32_t)input[3] << 24);
}

uint32_t jamscript_native_game_replay_v1(const uint8_t *input,
                                         uint32_t input_len,
                                         uint64_t *output) {
  if (input == 0 || output == 0 || input_len < 12u) return 1u;
  uint32_t version = read_u32(input);
  uint32_t health = read_u32(input + 4);
  uint32_t step_count = read_u32(input + 8);
  if (version != GAME_ENGINE_VERSION) return 2u;
  if (health == 0u || health > 1000u) return 3u;
  if (step_count > MAX_STEPS) return 4u;
  if (step_count > (input_len - 12u) / 4u) return 5u;
  if (input_len != 12u + step_count * 4u) return 5u;

  uint64_t score = (uint64_t)health;
  for (uint32_t index = 0; index < step_count; ++index) {
    const uint8_t *step = input + 12u + index * 4u;
    uint32_t opcode = step[0];
    uint32_t amount = step[1];
    if (step[2] != 0u || step[3] != 0u || amount == 0u) return 6u;
    if (opcode == OP_MOVE) {
      score += (uint64_t)amount;
    } else if (opcode == OP_ATTACK) {
      if (amount > health) return 7u;
      health -= amount;
      score += (uint64_t)amount * 2u;
    } else if (opcode == OP_BONUS) {
      if (health == 0u) return 8u;
      score += (uint64_t)amount * 3u;
    } else {
      return 9u;
    }
  }
  *output = score;
  return 0u;
}
