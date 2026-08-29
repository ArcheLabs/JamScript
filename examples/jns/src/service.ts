import { action, wallet, stateMap, query, string, address, record, u32 } from "jam";

const Name = string(32);
const JnsRecord = record({ owner: address, serviceId: u32 });

const names = stateMap({
    schema: "jns.names/v1",
    key: Name,
    value: JnsRecord,
});
const reverse = stateMap({
    schema: "jns.reverse/v1",
    key: address,
    value: Name,
});

export const claim = action({
    auth: wallet(),
    input: { name: Name, serviceId: u32 },
    execute(ctx, input) {
        if (names.has(input.name)) throw new Error("NAME_TAKEN");
        names.set(input.name, { owner: ctx.sender, serviceId: input.serviceId });
        reverse.set(ctx.sender, input.name);
    },
});

export const bind = action({
    auth: wallet(),
    input: { name: Name, serviceId: u32 },
    execute(ctx, input) {
        const current = names.get(input.name);
        if (!current || current.owner !== ctx.sender) throw new Error("NOT_OWNER");
        names.set(input.name, { owner: ctx.sender, serviceId: input.serviceId });
    },
});

export const resolve = query(names);
export const reverseLookup = query(reverse);
