#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

extern void jamscript_numeric_narrow_init(void);
extern void jamscript_numeric_narrow_execute_u8(const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t **, size_t *);
extern void jamscript_numeric_narrow_execute_u16(const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t **, size_t *);
extern void jamscript_numeric_narrow_execute_u32(const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t **, size_t *);

static int same(const uint8_t *actual, size_t actual_len, const uint8_t *expected, size_t expected_len) {
  return actual_len == expected_len && memcmp(actual, expected, expected_len) == 0;
}

static int run_u8(uint8_t left, uint8_t right, uint8_t operation, const uint8_t *expected, size_t expected_len) {
  const uint8_t payload[3] = {left, right, operation};
  const uint8_t empty = 0; const uint8_t *out = NULL; size_t out_len = 0;
  jamscript_numeric_narrow_execute_u8(payload, sizeof payload, &empty, 0, &empty, 0, &out, &out_len);
  return same(out, out_len, expected, expected_len);
}

static int run_u16(uint16_t left, uint16_t right, uint8_t operation, const uint8_t *expected, size_t expected_len) {
  const uint8_t payload[5] = {(uint8_t)left, (uint8_t)(left >> 8), (uint8_t)right, (uint8_t)(right >> 8), operation};
  const uint8_t empty = 0; const uint8_t *out = NULL; size_t out_len = 0;
  jamscript_numeric_narrow_execute_u16(payload, sizeof payload, &empty, 0, &empty, 0, &out, &out_len);
  return same(out, out_len, expected, expected_len);
}

static int run_u32(uint32_t left, uint32_t right, uint8_t operation, const uint8_t *expected, size_t expected_len) {
  const uint8_t payload[9] = {
    (uint8_t)left, (uint8_t)(left >> 8), (uint8_t)(left >> 16), (uint8_t)(left >> 24),
    (uint8_t)right, (uint8_t)(right >> 8), (uint8_t)(right >> 16), (uint8_t)(right >> 24), operation,
  };
  const uint8_t empty = 0; const uint8_t *out = NULL; size_t out_len = 0;
  jamscript_numeric_narrow_execute_u32(payload, sizeof payload, &empty, 0, &empty, 0, &out, &out_len);
  return same(out, out_len, expected, expected_len);
}

static void encode16(uint16_t value, uint8_t output[2]) { output[0] = (uint8_t)value; output[1] = (uint8_t)(value >> 8); }
static void encode32(uint32_t value, uint8_t output[4]) { for (unsigned i = 0; i < 4; i++) output[i] = (uint8_t)(value >> (8 * i)); }

static uint32_t next_random(uint32_t *state) {
  *state ^= *state << 13; *state ^= *state >> 17; *state ^= *state << 5; return *state;
}

int main(void) {
  const uint8_t fatal_overflow[6] = {1, 3, 3, 0, 0, 0x80};
  const uint8_t fatal_underflow[6] = {1, 3, 4, 0, 0, 0x80};
  const uint8_t fatal_div_zero[6] = {1, 3, 5, 0, 0, 0x80};
  const uint8_t less[1] = {0xff}; const uint8_t equal[1] = {0}; const uint8_t greater[1] = {1};
  jamscript_numeric_narrow_init();
  if (!run_u8(255, 1, 0, fatal_overflow, sizeof fatal_overflow) || !run_u8(16, 16, 2, fatal_overflow, sizeof fatal_overflow) || !run_u8(0, 1, 1, fatal_underflow, sizeof fatal_underflow) || !run_u8(1, 0, 3, fatal_div_zero, sizeof fatal_div_zero) || !run_u8(1, 1, 5, equal, 1) || !run_u8(2, 1, 5, greater, 1)) return 1;
  if (!run_u16(65535, 1, 0, fatal_overflow, sizeof fatal_overflow) || !run_u16(256, 256, 2, fatal_overflow, sizeof fatal_overflow) || !run_u16(0, 1, 1, fatal_underflow, sizeof fatal_underflow) || !run_u16(1, 0, 3, fatal_div_zero, sizeof fatal_div_zero) || !run_u16(1, 1, 5, equal, 1) || !run_u16(2, 1, 5, greater, 1)) return 2;
  if (!run_u32(UINT32_MAX, 1, 0, fatal_overflow, sizeof fatal_overflow) || !run_u32(65536, 65536, 2, fatal_overflow, sizeof fatal_overflow) || !run_u32(0, 1, 1, fatal_underflow, sizeof fatal_underflow) || !run_u32(1, 0, 3, fatal_div_zero, sizeof fatal_div_zero) || !run_u32(1, 1, 5, equal, 1) || !run_u32(2, 1, 5, greater, 1)) return 3;

  uint32_t random = 0x9e3779b9;
  for (unsigned index = 0; index < 1000; index++) {
    uint16_t a8 = (uint16_t)(next_random(&random) & 0xff); uint16_t b8 = (uint16_t)(next_random(&random) & (255 - a8));
    const uint8_t out8 = (uint8_t)(a8 + b8);
    if (!run_u8((uint8_t)a8, (uint8_t)b8, 0, &out8, 1)) return 10;
    if (!run_u8((uint8_t)(a8 + b8), (uint8_t)b8, 1, (const uint8_t[]){(uint8_t)a8}, 1)) return 11;
    const uint8_t div8 = (uint8_t)(b8 + 1); const uint8_t q8 = (uint8_t)(a8 / div8); const uint8_t r8 = (uint8_t)(a8 % div8);
    if (!run_u8((uint8_t)a8, div8, 3, &q8, 1) || !run_u8((uint8_t)a8, div8, 4, &r8, 1)) return 12;
    const uint8_t compare8 = (uint8_t)(index % 255); if (!run_u8(compare8, (uint8_t)(compare8 + 1), 5, less, 1)) return 13;

    uint32_t a16 = next_random(&random) & 0xffff; uint32_t b16 = next_random(&random) % (65536 - a16);
    uint8_t expected16[2]; encode16((uint16_t)(a16 + b16), expected16);
    if (!run_u16((uint16_t)a16, (uint16_t)b16, 0, expected16, 2)) return 20;
    encode16((uint16_t)a16, expected16);
    if (!run_u16((uint16_t)(a16 + b16), (uint16_t)b16, 1, expected16, 2)) return 21;
    uint16_t div16 = (uint16_t)(b16 + 1); encode16((uint16_t)(a16 / div16), expected16);
    if (!run_u16((uint16_t)a16, div16, 3, expected16, 2)) return 22;
    encode16((uint16_t)(a16 % div16), expected16);
    if (!run_u16((uint16_t)a16, div16, 4, expected16, 2)) return 23;
    const uint16_t compare16 = (uint16_t)(index % 65535); if (!run_u16(compare16, (uint16_t)(compare16 + 1), 5, less, 1)) return 24;

    uint32_t a32 = next_random(&random); uint32_t available32 = UINT32_MAX - a32; uint32_t b32 = available32 == 0 ? 0 : next_random(&random) % available32;
    uint8_t expected32[4]; encode32(a32 + b32, expected32);
    if (!run_u32(a32, b32, 0, expected32, 4)) return 30;
    encode32(a32, expected32);
    if (!run_u32(a32 + b32, b32, 1, expected32, 4)) return 31;
    uint32_t div32 = b32 + 1; encode32(a32 / div32, expected32);
    if (!run_u32(a32, div32, 3, expected32, 4)) return 32;
    encode32(a32 % div32, expected32);
    if (!run_u32(a32, div32, 4, expected32, 4)) return 33;
    const uint32_t compare32 = index; if (!run_u32(compare32, compare32 + 1, 5, less, 1)) return 34;

    uint16_t multiplicand = (uint16_t)(next_random(&random) % 16); uint16_t multiplier_limit = multiplicand == 0 ? 256 : (uint16_t)(255 / multiplicand + 1); uint16_t multiplier = (uint16_t)(next_random(&random) % multiplier_limit);
    const uint8_t product8 = (uint8_t)(multiplicand * multiplier);
    if (!run_u8((uint8_t)multiplicand, (uint8_t)multiplier, 2, &product8, 1)) return 40;
    encode16((uint16_t)((multiplicand & 0xff) * (multiplier & 0xff)), expected16);
    if (!run_u16((uint16_t)(multiplicand & 0xff), (uint16_t)(multiplier & 0xff), 2, expected16, 2)) return 41;
    const uint32_t multiplicand32 = next_random(&random) & 0xffff; const uint32_t multiplier32 = next_random(&random) & 0xffff;
    const uint32_t product32 = multiplicand32 * multiplier32;
    encode32(product32, expected32);
    if (!run_u32(multiplicand32, multiplier32, 2, expected32, 4)) return 42;
  }
  puts("SCRIPTC_FIXED_WIDTH_NUMERIC=PASS (18000 oracle cases)");
  return 0;
}
