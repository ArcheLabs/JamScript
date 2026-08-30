#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

extern void jamscript_m2_conformance_init(void);
extern void jamscript_m2_conformance_execute(
    const uint8_t *payload, size_t payload_len,
    const uint8_t *sender, size_t sender_len,
    const uint8_t *state, size_t state_len,
    const uint8_t **out, size_t *out_len);

static int expect(const uint8_t *actual, size_t actual_len,
                  const uint8_t *expected, size_t expected_len) {
  return actual_len == expected_len && memcmp(actual, expected, expected_len) == 0;
}

int main(void) {
  const uint8_t key[] = {1, 1};
  const uint8_t sender[] = {9};
  const uint8_t empty_view[] = {1, 0, 0, 0, 0};
  const uint8_t absent_view[] = {1, 1, 0, 0, 0, 2, 0, 0, 0, 1, 1, 0};
  const uint8_t need_state[] = {1, 2, 2, 0, 0, 0, 1, 1};
  const uint8_t applied[] = {
      1, 0, 17, 0, 0, 0,
      1, 1, 0, 0, 0,
      2, 0, 0, 0, 1, 1,
      1, 1, 0, 0, 0, 9};
  const uint8_t *out = NULL;
  size_t out_len = 0;

  jamscript_m2_conformance_init();
  jamscript_m2_conformance_execute(key, sizeof key, sender, sizeof sender,
                                   empty_view, sizeof empty_view, &out, &out_len);
  if (!expect(out, out_len, need_state, sizeof need_state)) return 1;

  jamscript_m2_conformance_execute(key, sizeof key, sender, sizeof sender,
                                   absent_view, sizeof absent_view, &out, &out_len);
  if (!expect(out, out_len, applied, sizeof applied)) return 2;

  puts("ScriptC M2 state execution: PASS");
  return 0;
}
