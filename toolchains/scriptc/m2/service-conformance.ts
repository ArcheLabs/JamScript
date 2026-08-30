import { action, wallet, stateMap, bytes, address, u32, record, abort } from "jam";

const Key = bytes(32);
const Entry = record({ owner: address, value: u32 });
const entries = stateMap({ schema: "test.entries/v1", key: Key, value: Entry });

function sameAddress(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

export const create = action({
  auth: wallet(),
  input: { key: Key, value: u32 },
  execute(ctx, input) {
    if (entries.has(input.key)) abort(1);
    entries.set(input.key, { owner: ctx.sender, value: input.value });
  },
});

export const update = action({
  auth: wallet(),
  input: { key: Key, value: u32 },
  execute(ctx, input) {
    const current = entries.get(input.key);
    if (!current) abort(2);
    if (!sameAddress(current.owner, ctx.sender)) abort(3);
    entries.set(input.key, { owner: current.owner, value: input.value });
  },
});
