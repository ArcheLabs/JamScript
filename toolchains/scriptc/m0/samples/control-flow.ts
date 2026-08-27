export function sum(n: number): number {
  let x = 0;
  for (let i = 0; i < n; i++) {
    if (i % 2 === 0) {
      x += i;
    }
  }
  return x;
}
