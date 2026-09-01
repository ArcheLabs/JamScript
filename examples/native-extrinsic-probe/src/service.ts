import { action, wallet, fixedBytes, stateMap, query, address, u32 } from "jam";
import { verifyProbe } from "native:probe";

const probeResult = stateMap({ schema: "native-extrinsic-probe/result/v1", key: address, value: u32 });

export const submitProbe = action({
  auth: wallet(),
  input: { runId: fixedBytes(32) },
  execute(ctx, input) {
    probeResult.set(ctx.sender, verifyProbe(input.runId));
  },
});

export const getProbeResult = query(probeResult);
