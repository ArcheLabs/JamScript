function helper(value: number): number {
  return value + 1;
}

function nested(value: number): number {
  return helper(value) * 2;
}

export function add(a: number, b: number): number {
  let sum = 0;
  for (let index = 0; index < 3; index++) {
    if (index > 0) sum += index;
  }
  let cursor = 0;
  while (cursor < 2) {
    sum += cursor;
    cursor++;
  }
  const comparison = nested(sum) >= 10 && a < b;
  const valid = comparison && !(a > b);
  return valid ? a + b : 0;
}

export function arrays(a: number, b: number): number {
  const values = [a, b, 1];
  return values[0] + values[1] + values[2];
}

export function objects(value: number): number {
  const record = { value, ready: true };
  return record.ready ? record.value : 0;
}

export function strings(value: number): number {
  const text = "jam";
  return text.length + value;
}

export function uint8array(value: number): number {
  const bytes = new Uint8Array(2);
  bytes[0] = 20;
  bytes[1] = 22;
  return bytes[0] + bytes[1] + value;
}
