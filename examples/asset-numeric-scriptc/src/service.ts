import { action, abort, wallet, state, stateMap, fixedBytes, record, u8, u64, u128 } from "jam";

const IdentityId = fixedBytes(32);

const AssetInfo = record({
  decimals: u8,
  totalSupply: u128,
});

const assetInfo = state({
  schema: "asset.info/v1",
  value: AssetInfo,
});

const balances = stateMap({
  schema: "asset.balance/v1",
  key: IdentityId,
  value: u128,
});

const nonces = stateMap({
  schema: "asset.nonce/v1",
  key: IdentityId,
  value: u64,
});

function addAmount(left: u128, right: u128): u128 {
  return left + right;
}

export const mint = action({
  auth: wallet(),
  input: { owner: IdentityId, amount: u128 },
  execute(ctx, input) {
    let info = assetInfo.get();
    if (!info) info = { decimals: 18, totalSupply: 0n };
    const current = balances.get(input.owner) ?? 0n;
    assetInfo.set({ decimals: info.decimals, totalSupply: addAmount(info.totalSupply, input.amount) });
    balances.set(input.owner, addAmount(current, input.amount));
  },
});

export const transfer = action({
  auth: wallet(),
  input: { from: IdentityId, to: IdentityId, amount: u128 },
  execute(ctx, input) {
    const fromBalance = balances.get(input.from) ?? 0n;
    const toBalance = balances.get(input.to) ?? 0n;
    if (fromBalance < input.amount) abort(1);
    balances.set(input.from, fromBalance - input.amount);
    balances.set(input.to, toBalance + input.amount);
    const nonce = nonces.get(input.from) ?? 0n;
    nonces.set(input.from, nonce + 1n);
  },
});

export const burn = action({
  auth: wallet(),
  input: { owner: IdentityId, amount: u128 },
  execute(ctx, input) {
    const info = assetInfo.get();
    if (!info) abort(2);
    const current = balances.get(input.owner) ?? 0n;
    if (current < input.amount) abort(1);
    assetInfo.set({ decimals: info.decimals, totalSupply: info.totalSupply - input.amount });
    balances.set(input.owner, current - input.amount);
  },
});
