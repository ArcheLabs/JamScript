import { action, publicAction, u64 } from "jam";

export const increment = action({
  auth: publicAction(),
  input: { value: u64 },
  execute(ctx, input) {
    return input.value + 1;
  },
});
