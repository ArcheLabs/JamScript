import { action, publicAction, u64 } from "jam";

// This fixture is intentionally outside the workspace examples. It models
// the smallest external consumer that still exercises ScriptC, the JAM SDK,
// the PolkaVM cdylib guest, and compiler-builtins through build-std.
export const hello = action({
  auth: publicAction(),
  input: { value: u64 },
  execute(_ctx, input) {
    return input.value + 1;
  },
});
