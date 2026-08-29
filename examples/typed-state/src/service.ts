import { action, wallet, stateMap, query, string, address, record, u64, bool } from "jam";

const Profile = record({
    name: string(32),
    score: u64,
    active: bool,
});

const profiles = stateMap({
    schema: "profile/v1",
    key: address,
    value: Profile,
});

export const updateProfile = action({
    auth: wallet(),
    input: { name: string(32) },
    execute(ctx, input) {
        const old = profiles.get(ctx.sender);
        profiles.set(ctx.sender, {
            name: input.name,
            score: old ? old.score : 0n,
            active: true,
        });
    },
});

export const getProfile = query(profiles);
