import { abort, action, ownership, ownershipKey, stateMap, fixedBytes, publicAction, u8, u128, verifyEd25519 } from "jam";

const OwnershipKey = fixedBytes(32);
const balances = stateMap({
  schema: "release.ownership.balances/v1",
  key: OwnershipKey,
  value: u128,
});

const PublicKey = fixedBytes(32);
const Message = fixedBytes(32);
const Signature = fixedBytes(64);

export const probe = action({
  auth: publicAction(),
  input: { stage: u8, publicKey: PublicKey, message: Message, signature: Signature },
  execute(_ctx, input) {
    if (input.stage === 0) abort(5098);
    if (!verifyEd25519(input.publicKey, input.message, input.signature)) abort(5005);
    abort(5098);
  },
});

export const credit = action({
  auth: ownership(),
  input: { to: ownership, amount: u128 },
  execute(ctx, input) {
    const controllerKey = ownershipKey(ctx.controller);
    const ownerKey = ownershipKey(ctx.owner);
    const recipientKey = ownershipKey(input.to);
    balances.set(ownerKey, input.amount);
    balances.set(recipientKey, input.amount);
    balances.set(controllerKey, input.amount);
  },
});

export const transfer = action({
  auth: ownership(),
  input: { to: ownership, amount: u128 },
  execute(ctx, input) {
    const senderKey = ownershipKey(ctx.owner);
    const recipientKey = ownershipKey(input.to);
    balances.set(senderKey, input.amount);
    balances.set(recipientKey, input.amount);
  },
});
