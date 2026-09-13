import {
  caughtResult,
} from "../runtime.js";
import {
  jamDecodeU128,
  jamEncodeU128,
  jamU128AddChecked,
  jamU128SubChecked,
  jamU128MulChecked,
  jamU128DivChecked,
  jamU128ModChecked,
  jamU128Compare,
} from "./runtime.js";

export function execute(payload: Uint8Array, _sender: Uint8Array, _state: Uint8Array): Uint8Array {
  try {
    if (payload.length !== 33) throw new Error("numeric conformance payload length");
    const left = jamDecodeU128(payload, 0);
    const right = jamDecodeU128(payload, 16);
    const operation = payload[32];
    if (operation === 0) return jamEncodeU128(jamU128AddChecked(left, right));
    if (operation === 1) return jamEncodeU128(jamU128SubChecked(left, right));
    if (operation === 2) return jamEncodeU128(jamU128MulChecked(left, right));
    if (operation === 3) return jamEncodeU128(jamU128DivChecked(left, right));
    if (operation === 4) return jamEncodeU128(jamU128ModChecked(left, right));
    const comparison = jamU128Compare(left, right);
    return new Uint8Array([comparison < 0 ? 255 : comparison > 0 ? 1 : 0]);
  } catch (error) {
    return caughtResult(error);
  }
}
