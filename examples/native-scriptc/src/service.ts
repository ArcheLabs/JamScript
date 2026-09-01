import { action, wallet, bytes, stateMap, address, u32 } from "jam";
import { calculate } from "native:math";

const result = stateMap({
  schema: "native-scriptc/result/v1",
  key: address,
  value: u32,
});

export const run = action({
  auth: wallet(),
  input: { payload: bytes(256) },
  execute(ctx, input) {
    result.set(ctx.sender, calculate(input.payload));
  },
});
