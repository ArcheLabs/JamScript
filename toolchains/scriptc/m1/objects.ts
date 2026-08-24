export function project(value: number): number {
  const record = { value: value, ready: true };
  return record.ready ? record.value : 0;
}
