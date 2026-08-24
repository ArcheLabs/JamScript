export function sum(): number {
  const values = [1, 2, 3];
  let x = 0;
  for (const value of values) {
    x += value;
  }
  return x;
}
