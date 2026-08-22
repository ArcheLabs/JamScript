import { action, wallet, u64 } from "jam";

export const increment = action({
  auth: wallet(),
  input: { value: u64 },
  execute(ctx, input) {
    return input.value + 1;
  },
});
