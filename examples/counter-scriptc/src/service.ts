import { action, wallet, u64 } from "jam";

function incrementValue(value: number): number {
  let result = value;
  if (value >= 0) {
    let step = 0;
    while (step < 1) {
      result += 1;
      step += 1;
    }
  } else {
    result -= 1;
  }
  return result;
}

export const increment = action({
  auth: wallet(),
  input: { value: u64 },
  execute(ctx, input) {
    return incrementValue(input.value);
  },
});
