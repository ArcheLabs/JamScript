import { action, wallet, stateMap, query, u64, bytes, address } from "jam";
import { replay } from "native:game";

const bestScore = stateMap({
  schema: "best-score/v1",
  key: address,
  value: u64,
});

export const submitRun = action({
  auth: wallet(),
  input: {
    run: bytes(262144),
  },
  compute(ctx, input) {
    return replay(input.run);
  },
  commit(ctx, score) {
    bestScore.max(ctx.sender, score);
  },
});

export const getBestScore = query(bestScore);
