import { abort, action, fixedBytes, ownership, ownershipKey, record, stateMap, u8 } from "jam";

const OwnerKey = fixedBytes(32);
const ControllerGrantKey = record({
  subjectKey: OwnerKey,
  controllerKey: OwnerKey,
});

const controllerGrants = stateMap({
  schema: "test.controller-grant.v1",
  key: ControllerGrantKey,
  value: u8,
});

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

function sameOwnership(left: JamOwnership, right: JamOwnership): boolean {
  return left.version === right.version
    && left.kind === right.kind
    && sameBytes(left.public, right.public);
}

function requireController(subject: JamOwnership, controller: JamOwnership): void {
  if (sameOwnership(subject, controller)) return;
  const key = {
    subjectKey: ownershipKey(subject),
    controllerKey: ownershipKey(controller),
  };
  if ((controllerGrants.get(key) ?? 0) !== 1) abort(5001);
}

export const directOwnership = action({
  auth: ownership(),
  input: {},
  execute(_ctx, _input) {},
});

// INV-MATRIX-DELEGATED-OWNERSHIP-001:
// A verified Matrix device D must be able to execute ordinary ownership
// actions for an authorized subject M with SignedActionV2, without replaying
// the M→S→D bootstrap proof and without an unclassified PVM trap.
export const delegatedOwnership = action({
  auth: ownership(),
  input: { subject: ownership },
  execute(ctx, input) {
    requireController(input.subject, ctx.controller);
  },
});
