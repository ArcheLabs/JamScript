#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef unsigned __int128 u128;

extern void jamscript_numeric_conformance_init(void);
extern void jamscript_numeric_conformance_execute(
    const uint8_t *payload, size_t payload_len,
    const uint8_t *sender, size_t sender_len,
    const uint8_t *state, size_t state_len,
    const uint8_t **out, size_t *out_len);

static int expect(const uint8_t *actual, size_t actual_len,
                  const uint8_t *expected, size_t expected_len) {
  return actual_len == expected_len && memcmp(actual, expected, expected_len) == 0;
}

static int run(const uint8_t *left, const uint8_t *right, uint8_t operation,
               const uint8_t *expected, size_t expected_len) {
  uint8_t payload[33];
  memcpy(payload, left, 16);
  memcpy(payload + 16, right, 16);
  payload[32] = operation;
  const uint8_t *out = NULL;
  size_t out_len = 0;
  const uint8_t empty = 0;
  jamscript_numeric_conformance_execute(payload, sizeof payload, &empty, 0, &empty, 0, &out, &out_len);
  if (!expect(out, out_len, expected, expected_len)) {
    fprintf(stderr, "numeric op %u mismatch (%zu):", operation, out_len);
    for (size_t index = 0; index < out_len; index++) fprintf(stderr, " %02x", out[index]);
    fputc('\n', stderr);
    return 0;
  }
  return 1;
}

static uint64_t next_random(uint64_t *state) {
  *state ^= *state << 7;
  *state ^= *state >> 9;
  *state ^= *state << 8;
  return *state;
}

static void store_u128(u128 value, uint8_t output[16]) {
  for (size_t index = 0; index < 16; index++) {
    output[index] = (uint8_t)(value & 0xff);
    value >>= 8;
  }
}

static int run_oracle(u128 left, u128 right, uint8_t operation) {
  uint8_t left_bytes[16];
  uint8_t right_bytes[16];
  uint8_t expected[16];
  store_u128(left, left_bytes);
  store_u128(right, right_bytes);
  if (operation == 0) store_u128(left + right, expected);
  else if (operation == 1) store_u128(left - right, expected);
  else if (operation == 2) store_u128(left * right, expected);
  else if (operation == 3) store_u128(left / right, expected);
  else if (operation == 4) store_u128(left % right, expected);
  else {
    const uint8_t comparison = left < right ? 0xff : left > right ? 1 : 0;
    return run(left_bytes, right_bytes, operation, &comparison, sizeof comparison);
  }
  return run(left_bytes, right_bytes, operation, expected, sizeof expected);
}

int main(void) {
  const uint8_t zero[16] = {0};
  const uint8_t one[16] = {1};
  const uint8_t two[16] = {2};
  const uint8_t three[16] = {3};
  const uint8_t six[16] = {6};
  const uint8_t max[16] = {0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                           0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff};
  const uint8_t two64[16] = {0, 0, 0, 0, 0, 0, 0, 0, 1};
  const uint8_t one_u128[16] = {1};
  const uint8_t three_u128[16] = {3};
  const uint8_t quotient[16] = {0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55};
  const uint8_t remainder[16] = {1};
  const uint8_t fatal_overflow[6] = {1, 3, 3, 0, 0, 0x80};
  const uint8_t fatal_underflow[6] = {1, 3, 4, 0, 0, 0x80};
  const uint8_t fatal_div_zero[6] = {1, 3, 5, 0, 0, 0x80};
  const uint8_t comparison_less[1] = {0xff};
  const uint8_t comparison_equal[1] = {0};
  const uint8_t comparison_greater[1] = {1};

  jamscript_numeric_conformance_init();
  if (!run(one, two, 0, three, 16)) return 1;
  if (!run(three, one, 1, two, 16)) return 2;
  if (!run(two, three, 2, six, 16)) return 3;
  if (!run(two64, three_u128, 3, quotient, 16)) return 4;
  if (!run(two64, three_u128, 4, remainder, 16)) return 5;
  if (!run(max, one_u128, 0, fatal_overflow, sizeof fatal_overflow)) return 6;
  if (!run(zero, one_u128, 1, fatal_underflow, sizeof fatal_underflow)) return 7;
  if (!run(one_u128, zero, 3, fatal_div_zero, sizeof fatal_div_zero)) return 8;
  if (!run(one, two, 5, comparison_less, sizeof comparison_less)) return 9;
  if (!run(two, two, 5, comparison_equal, sizeof comparison_equal)) return 10;
  if (!run(two, one, 5, comparison_greater, sizeof comparison_greater)) return 11;

  // Exercise every checked wide operation with deterministic oracle vectors.
  // The operands are chosen so the expected result stays in u128 for the
  // valid cases; overflow, underflow, and division-by-zero are tested above.
  uint64_t random = 0x4d595df4d0f33173ULL;
  for (unsigned index = 0; index < 1000; index++) {
    const u128 high = (u128)((next_random(&random) & 0x7fffffffffffffffULL) | 1);
    const u128 left = (high << 64) | next_random(&random);
    const u128 right = (u128)(next_random(&random) % 1000000ULL) + 1;
    if (!run_oracle(left, right, 0)) return 20;
    if (!run_oracle(left, right, 1)) return 21;
    if (!run_oracle(left, right, 3)) return 22;
    if (!run_oracle(left, right, 4)) return 23;
    if (!run_oracle(left, right, 5)) return 24;

    const u128 multiplicand = (u128)next_random(&random);
    const u128 multiplier = (u128)next_random(&random);
    if (!run_oracle(multiplicand, multiplier, 2)) return 25;
  }
  puts("SCRIPTC_FIXED_LIMB_NUMERIC_VECTORS=PASS (6000 oracle cases)");
  puts("ScriptC fixed-limb numeric conformance: PASS");
  return 0;
}
