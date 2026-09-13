import { failNumeric } from "../runtime.js";

export type JamU64 = { w0: number; w1: number };
export type JamU128 = { w0: number; w1: number; w2: number; w3: number };

const LIMB_BASE = 4294967296;
const HALF_BASE = 65536;
const MAX_LIMB = 4294967295;

function checkedLimb(value: number): number {
  if (value < 0 || value > MAX_LIMB || Math.floor(value) !== value) { failNumeric(0x80000006); return 0; }
  return value;
}

function checkedNumber(value: number): number {
  if (value < 0 || value > 9007199254740991 || Math.floor(value) !== value) { failNumeric(0x80000006); return 0; }
  return value;
}

function copyWords(value: number[], length: number): number[] {
  const output: number[] = [];
  for (let index = 0; index < length; index += 1) output.push(checkedLimb(value[index]));
  return output;
}

function parseDecimal(text: string, length: number): number[] {
  if (text.length === 0) { failNumeric(0x80000006); return []; }
  const words: number[] = [];
  for (let index = 0; index < length; index += 1) words.push(0);
  for (let position = 0; position < text.length; position += 1) {
    const digit = text.charCodeAt(position) - 48;
    if (digit < 0 || digit > 9) { failNumeric(0x80000006); return []; }
    let carry = digit;
    for (let index = 0; index < length; index += 1) {
      const product = words[index] * 10 + carry;
      words[index] = product % LIMB_BASE;
      carry = Math.floor(product / LIMB_BASE);
    }
    if (carry !== 0) { failNumeric(0x80000003); return []; }
  }
  return words;
}

function wordsToU64(words: number[]): JamU64 { return { w0: words[0], w1: words[1] }; }
function wordsToU128(words: number[]): JamU128 { return { w0: words[0], w1: words[1], w2: words[2], w3: words[3] }; }
function u64Words(value: JamU64): number[] { return copyWords([value.w0, value.w1], 2); }
function u128Words(value: JamU128): number[] { return copyWords([value.w0, value.w1, value.w2, value.w3], 4); }

export function jamU64Const(text: string): JamU64 { return wordsToU64(parseDecimal(text, 2)); }
export function jamU128Const(text: string): JamU128 { return wordsToU128(parseDecimal(text, 4)); }
export function jamU64Identity(value: JamU64): JamU64 { return wordsToU64(u64Words(value)); }
export function jamU128Identity(value: JamU128): JamU128 { return wordsToU128(u128Words(value)); }
export function jamU64Or(left: JamU64 | null, right: JamU64): JamU64 { return left === null ? jamU64Identity(right) : jamU64Identity(left); }
export function jamU128Or(left: JamU128 | null, right: JamU128): JamU128 { return left === null ? jamU128Identity(right) : jamU128Identity(left); }

function addWords(left: number[], right: number[]): number[] {
  const output: number[] = [];
  let carry = 0;
  for (let index = 0; index < left.length; index += 1) {
    let sum = left[index] + right[index] + carry;
    if (sum >= LIMB_BASE) { sum -= LIMB_BASE; carry = 1; } else carry = 0;
    output.push(sum);
  }
  if (carry !== 0) { failNumeric(0x80000003); return []; }
  return output;
}

function subWords(left: number[], right: number[]): number[] {
  const output: number[] = [];
  let borrow = 0;
  for (let index = 0; index < left.length; index += 1) {
    let difference = left[index] - right[index] - borrow;
    if (difference < 0) { difference += LIMB_BASE; borrow = 1; } else borrow = 0;
    output.push(difference);
  }
  if (borrow !== 0) { failNumeric(0x80000004); return []; }
  return output;
}

function compareWords(left: number[], right: number[]): number {
  for (let index = left.length - 1; index >= 0; index -= 1) {
    if (left[index] < right[index]) return -1;
    if (left[index] > right[index]) return 1;
  }
  return 0;
}

function multiplyWords(left: number[], right: number[]): number[] {
  const digits = left.length * 2;
  const a: number[] = [];
  const b: number[] = [];
  for (let index = 0; index < left.length; index += 1) {
    a.push(left[index] % HALF_BASE, Math.floor(left[index] / HALF_BASE));
    b.push(right[index] % HALF_BASE, Math.floor(right[index] / HALF_BASE));
  }
  const output: number[] = [];
  for (let index = 0; index < digits; index += 1) output.push(0);
  for (let i = 0; i < digits; i += 1) {
    let carry = 0;
    for (let j = 0; j < digits; j += 1) {
      const position = i + j;
      if (position >= digits) {
        if (a[i] !== 0 && b[j] !== 0) { failNumeric(0x80000003); return []; }
        continue;
      }
      const value = output[position] + a[i] * b[j] + carry;
      output[position] = value % HALF_BASE;
      carry = Math.floor(value / HALF_BASE);
    }
    let position = i + digits;
    while (carry !== 0) {
      if (position >= digits) { failNumeric(0x80000003); return []; }
      const value = output[position] + carry;
      output[position] = value % HALF_BASE;
      carry = Math.floor(value / HALF_BASE);
      position += 1;
    }
  }
  const words: number[] = [];
  for (let index = 0; index < left.length; index += 1) words.push(output[index * 2] + output[index * 2 + 1] * HALF_BASE);
  return words;
}

function getBit(words: number[], bit: number): number {
  const word = Math.floor(bit / 32);
  const offset = bit % 32;
  const divisor = bitValue(offset);
  return Math.floor(words[word] / divisor) % 2;
}

function setBit(words: number[], bit: number): void {
  const word = Math.floor(bit / 32);
  const offset = bit % 32;
  const value = bitValue(offset);
  if (Math.floor(words[word] / value) % 2 === 0) words[word] = words[word] + value;
}

function bitValue(offset: number): number {
  let value = 1;
  let index = 0;
  while (index < offset) { value *= 2; index += 1; }
  return value;
}

function shiftLeftBit(words: number[], bit: number): void {
  let carry = bit;
  for (let index = 0; index < words.length; index += 1) {
    const next = Math.floor(words[index] / 2147483648);
    let value = words[index] * 2 + carry;
    if (value >= LIMB_BASE) value -= LIMB_BASE;
    words[index] = value;
    carry = next;
  }
}

function divModWords(dividend: number[], divisor: number[]): { quotient: number[]; remainder: number[] } {
  if (compareWords(divisor, divisor.map(() => 0)) === 0) { failNumeric(0x80000005); return { quotient: [], remainder: [] }; }
  const quotient: number[] = [];
  const remainder: number[] = [];
  for (let index = 0; index < dividend.length; index += 1) { quotient.push(0); remainder.push(0); }
  for (let bit = dividend.length * 32 - 1; bit >= 0; bit -= 1) {
    shiftLeftBit(remainder, getBit(dividend, bit));
    if (compareWords(remainder, divisor) >= 0) {
      const difference = subWords(remainder, divisor);
      for (let index = 0; index < remainder.length; index += 1) remainder[index] = difference[index];
      setBit(quotient, bit);
    }
  }
  return { quotient, remainder };
}

function encodeWords(words: number[]): Uint8Array {
  const output = new Uint8Array(words.length * 4);
  for (let index = 0; index < words.length; index += 1) {
    const word = checkedLimb(words[index]);
    output[index * 4] = word % 256;
    output[index * 4 + 1] = Math.floor(word / 256) % 256;
    output[index * 4 + 2] = Math.floor(word / 65536) % 256;
    output[index * 4 + 3] = Math.floor(word / 16777216) % 256;
  }
  return output;
}

function decodeWords(input: Uint8Array, offset: number, length: number): number[] {
  const words: number[] = [];
  for (let index = 0; index < length; index += 1) {
    const at = offset + index * 4;
    if (at + 4 > input.length) { failNumeric(0x80000006); return []; }
    words.push(input[at] + input[at + 1] * 256 + input[at + 2] * 65536 + input[at + 3] * 16777216);
  }
  return words;
}

export function jamU64AddChecked(left: JamU64, right: JamU64): JamU64 { return wordsToU64(addWords(u64Words(left), u64Words(right))); }
export function jamU64SubChecked(left: JamU64, right: JamU64): JamU64 { return wordsToU64(subWords(u64Words(left), u64Words(right))); }
export function jamU64MulChecked(left: JamU64, right: JamU64): JamU64 { return wordsToU64(multiplyWords(u64Words(left), u64Words(right))); }
export function jamU64DivChecked(left: JamU64, right: JamU64): JamU64 { return wordsToU64(divModWords(u64Words(left), u64Words(right)).quotient); }
export function jamU64ModChecked(left: JamU64, right: JamU64): JamU64 { return wordsToU64(divModWords(u64Words(left), u64Words(right)).remainder); }
export function jamU64Compare(left: JamU64, right: JamU64): number { return compareWords(u64Words(left), u64Words(right)); }

export function jamU128AddChecked(left: JamU128, right: JamU128): JamU128 { return wordsToU128(addWords(u128Words(left), u128Words(right))); }
export function jamU128SubChecked(left: JamU128, right: JamU128): JamU128 { return wordsToU128(subWords(u128Words(left), u128Words(right))); }
export function jamU128MulChecked(left: JamU128, right: JamU128): JamU128 { return wordsToU128(multiplyWords(u128Words(left), u128Words(right))); }
export function jamU128DivChecked(left: JamU128, right: JamU128): JamU128 { return wordsToU128(divModWords(u128Words(left), u128Words(right)).quotient); }
export function jamU128ModChecked(left: JamU128, right: JamU128): JamU128 { return wordsToU128(divModWords(u128Words(left), u128Words(right)).remainder); }
export function jamU128Compare(left: JamU128, right: JamU128): number { return compareWords(u128Words(left), u128Words(right)); }

function checkedSmall(value: number, maximum: number, underflow: boolean): number {
  if (value < 0) { failNumeric(underflow ? 0x80000004 : 0x80000003); return 0; }
  if (value > maximum || Math.floor(value) !== value) { failNumeric(0x80000003); return 0; }
  return value;
}
function smallAdd(left: number, right: number, maximum: number): number { return checkedSmall(left + right, maximum, false); }
function smallSub(left: number, right: number, maximum: number): number { return checkedSmall(left - right, maximum, true); }
function smallMul(left: number, right: number, maximum: number): number { return checkedSmall(left * right, maximum, false); }
function smallDiv(left: number, right: number, maximum: number): number { if (right === 0) { failNumeric(0x80000005); return 0; } return checkedSmall(Math.floor(left / right), maximum, false); }
function smallMod(left: number, right: number, maximum: number): number { if (right === 0) { failNumeric(0x80000005); return 0; } return checkedSmall(left % right, maximum, false); }
function smallCompare(left: number, right: number): number { return left < right ? -1 : left > right ? 1 : 0; }

export function jamU8AddChecked(left: number, right: number): number { return smallAdd(left, right, 255); }
export function jamU8SubChecked(left: number, right: number): number { return smallSub(left, right, 255); }
export function jamU8MulChecked(left: number, right: number): number { return smallMul(left, right, 255); }
export function jamU8DivChecked(left: number, right: number): number { return smallDiv(left, right, 255); }
export function jamU8ModChecked(left: number, right: number): number { return smallMod(left, right, 255); }
export function jamU8Compare(left: number, right: number): number { return smallCompare(left, right); }
export function jamU16AddChecked(left: number, right: number): number { return smallAdd(left, right, 65535); }
export function jamU16SubChecked(left: number, right: number): number { return smallSub(left, right, 65535); }
export function jamU16MulChecked(left: number, right: number): number { return smallMul(left, right, 65535); }
export function jamU16DivChecked(left: number, right: number): number { return smallDiv(left, right, 65535); }
export function jamU16ModChecked(left: number, right: number): number { return smallMod(left, right, 65535); }
export function jamU16Compare(left: number, right: number): number { return smallCompare(left, right); }
export function jamU32AddChecked(left: number, right: number): number { return smallAdd(left, right, MAX_LIMB); }
export function jamU32SubChecked(left: number, right: number): number { return smallSub(left, right, MAX_LIMB); }
export function jamU32MulChecked(left: number, right: number): number {
  if (left < 0 || right < 0 || left > MAX_LIMB || right > MAX_LIMB || Math.floor(left) !== left || Math.floor(right) !== right) { failNumeric(0x80000006); return 0; }
  const leftLow = left % HALF_BASE;
  const leftHigh = Math.floor(left / HALF_BASE);
  const rightLow = right % HALF_BASE;
  const rightHigh = Math.floor(right / HALF_BASE);
  const lowProduct = leftLow * rightLow;
  const middle = leftHigh * rightLow + leftLow * rightHigh + Math.floor(lowProduct / HALF_BASE);
  const high = leftHigh * rightHigh + Math.floor(middle / HALF_BASE);
  if (high !== 0) { failNumeric(0x80000003); return 0; }
  return (lowProduct % HALF_BASE) + (middle % HALF_BASE) * HALF_BASE;
}
export function jamU32DivChecked(left: number, right: number): number { return smallDiv(left, right, MAX_LIMB); }
export function jamU32ModChecked(left: number, right: number): number { return smallMod(left, right, MAX_LIMB); }
export function jamU32Compare(left: number, right: number): number { return smallCompare(left, right); }

export function jamU64FromNumber(value: number): JamU64 {
  const checked = checkedNumber(value);
  return { w0: checked % LIMB_BASE, w1: Math.floor(checked / LIMB_BASE) };
}
export function jamU128FromNumber(value: number): JamU128 {
  const checked = checkedNumber(value);
  const w0 = checked % LIMB_BASE;
  const w1 = Math.floor(checked / LIMB_BASE);
  return { w0, w1, w2: 0, w3: 0 };
}
export function jamU128FromU64(value: JamU64): JamU128 { return { w0: checkedLimb(value.w0), w1: checkedLimb(value.w1), w2: 0, w3: 0 }; }
export function jamU64FromU128(value: JamU128): JamU64 {
  if (value.w2 !== 0 || value.w3 !== 0) { failNumeric(0x80000006); return { w0: 0, w1: 0 }; }
  return { w0: checkedLimb(value.w0), w1: checkedLimb(value.w1) };
}

export function jamU64ToNumber(value: JamU64): number {
  if (value.w1 !== 0) { failNumeric(0x80000006); return 0; }
  return checkedLimb(value.w0);
}
export function jamU128ToNumber(value: JamU128): number {
  if (value.w1 !== 0 || value.w2 !== 0 || value.w3 !== 0) { failNumeric(0x80000006); return 0; }
  return checkedLimb(value.w0);
}
export function jamU8FromNumber(value: number): number { if (value < 0 || value > 255 || Math.floor(value) !== value) { failNumeric(0x80000006); return 0; } return value; }
export function jamU16FromNumber(value: number): number { if (value < 0 || value > 65535 || Math.floor(value) !== value) { failNumeric(0x80000006); return 0; } return value; }
export function jamU32FromNumber(value: number): number { if (value < 0 || value > MAX_LIMB || Math.floor(value) !== value) { failNumeric(0x80000006); return 0; } return value; }
export function jamU8FromU64(value: JamU64): number { return jamU8FromNumber(jamU64ToNumber(value)); }
export function jamU16FromU64(value: JamU64): number { return jamU16FromNumber(jamU64ToNumber(value)); }
export function jamU32FromU64(value: JamU64): number { return jamU32FromNumber(jamU64ToNumber(value)); }
export function jamU8FromU128(value: JamU128): number { return jamU8FromNumber(jamU128ToNumber(value)); }
export function jamU16FromU128(value: JamU128): number { return jamU16FromNumber(jamU128ToNumber(value)); }
export function jamU32FromU128(value: JamU128): number { return jamU32FromNumber(jamU128ToNumber(value)); }

export function jamEncodeU64(value: JamU64): Uint8Array { return encodeWords(u64Words(value)); }
export function jamEncodeU128(value: JamU128): Uint8Array { return encodeWords(u128Words(value)); }
export function jamDecodeU64(input: Uint8Array, offset: number): JamU64 { return wordsToU64(decodeWords(input, offset, 2)); }
export function jamDecodeU128(input: Uint8Array, offset: number): JamU128 { return wordsToU128(decodeWords(input, offset, 4)); }
