import { abort, action, fixedBytes, ownership, ownershipKey, publicAction, u8, verifyEd25519 } from "jam";

const PublicKey = fixedBytes(32);
const Signature = fixedBytes(64);

export const probe = action({
  auth: publicAction(),
  input: { stage: u8, subject: ownership, publicKey: PublicKey, signature: Signature },
  execute(_ctx, input) {
    if (input.stage === 0) abort(5098);
    if (!verifyEd25519(input.publicKey, ownershipKey(input.subject), input.signature)) abort(5005);
    abort(5098);
  },
});
