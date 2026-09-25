import {
  abort,
  action,
  fixedBytes,
  publicAction,
  record,
  u8,
  u16,
  verifyEd25519,
} from "jam";
import {
  verifyMatrixOwnershipAuthorizationScriptc,
} from "@jamscript/client/ownership/matrix/service-scriptc";

const JamOwnership = record({
  version: u8,
  kind: u8,
  public: fixedBytes(32),
});

export const probe = action({
  auth: publicAction(),
  input: {
    subject: JamOwnership,
    controller: JamOwnership,
    proofLength: u16,
    proof: fixedBytes(2048),
  },
  execute(_ctx, input) {
    const proof = input.proof.slice(0, input.proofLength);
    if (!verifyMatrixOwnershipAuthorizationScriptc(input.subject, input.controller, proof)) {
      abort(5005);
    }
    // A valid proof intentionally reaches this ordinary application abort.
    abort(5098);
  },
});
