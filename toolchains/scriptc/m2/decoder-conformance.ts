import { action, wallet, address, bytes, u8, u32, u64, u128 } from "jam";

export const fixedDynamicFixed = action({
  auth: wallet(),
  input: { head: u32, payload: bytes(8), tail: u128 },
  execute(_ctx, _input) {},
});

export const twoDynamicFields = action({
  auth: wallet(),
  input: { head: u8, name: bytes(64), symbol: bytes(16), tail: u128 },
  execute(_ctx, _input) {},
});

export const locusCreateAssetShape = action({
  auth: wallet(),
  input: {
    issuerId: address,
    nonce: u64,
    assetId: address,
    name: bytes(64),
    symbol: bytes(16),
    decimals: u8,
    initialSupply: u128,
  },
  execute(_ctx, _input) {},
});
