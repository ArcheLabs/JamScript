import { action, abort, wallet, stateMap, bytes, address, record, u32 } from "jam";

const IndexKey = bytes(32);

const IndexEntry = record({
  next: bytes(32),
});

const Value = record({
  owner: address,
  value: u32,
});

const index = stateMap({
  schema: "test.index/v1",
  key: IndexKey,
  value: IndexEntry,
});

const values = stateMap({
  schema: "test.values/v1",
  key: IndexKey,
  value: Value,
});

export const advance = action({
  auth: wallet(),
  input: { key: IndexKey },
  execute(ctx, input) {
    const pointer = index.get(input.key);
    if (!pointer) abort(1);

    const current = values.get(pointer.next);
    if (!current) abort(2);

    values.set(pointer.next, {
      owner: ctx.sender,
      value: current.value + 1,
    });
  },
});
