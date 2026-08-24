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
