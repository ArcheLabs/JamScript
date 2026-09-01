import { action, wallet, fixedBytes } from "jam";
import { verifyProbe } from "native:probe";

export const submitProbe = action({
  auth: wallet(),
  input: { runId: fixedBytes(32) },
  execute(_ctx, input) {
    verifyProbe(input.runId);
  },
});
