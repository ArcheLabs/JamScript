import { action, ownership, ownershipKey, stateMap, fixedBytes, u128 } from "jam";

const OwnershipKey = fixedBytes(32);
const balances = stateMap({
  schema: "release.ownership.balances/v1",
  key: OwnershipKey,
  value: u128,
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
