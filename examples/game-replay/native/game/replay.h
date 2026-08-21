#ifndef JAMSCRIPT_GAME_REPLAY_H
#define JAMSCRIPT_GAME_REPLAY_H

#include <stdint.h>

uint32_t jamscript_native_game_replay_v1(const uint8_t *input,
                                         uint32_t input_len,
                                         uint64_t *output);

#endif
