import { abort, action, fixedBytes, publicAction, u8 } from "jam";

const MatrixKey = fixedBytes(32);
const MatrixProof = fixedBytes(334);

export const probe = action({
  auth: publicAction(),
  input: {
    stage: u8,
    subject: MatrixKey,
    controller: MatrixKey,
    proof: MatrixProof,
  },
  execute(_ctx, input) {
    if (input.stage === 0) abort(5098);
    const subject = { version: 1, kind: 0, public: input.subject };
    const controller = { version: 1, kind: 0, public: input.controller };
    if (!verifyMatrixCrossSigning(subject, controller, input.proof)) abort(5005);
    abort(5098);
  },
});
