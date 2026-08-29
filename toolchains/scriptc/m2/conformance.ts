import {
  appliedResult,
  caughtResult,
  initializeStateView,
  stateGetRaw,
  stateSetRaw,
} from "./runtime.js";

export function execute(
  payload: Uint8Array,
  sender: Uint8Array,
  state: Uint8Array,
): Uint8Array {
  try {
    initializeStateView(state);
    const current = stateGetRaw(payload);
    if (current === null) {
      stateSetRaw(payload, sender);
      if (stateGetRaw(payload) === null) throw new Error("overlay read failed");
    }
    return appliedResult();
  } catch (error) {
    return caughtResult(error);
  }
}
