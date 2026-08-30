export type AbiTypeDescriptor =
  | { kind: "unit" | "bool" | "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" | "address" }
  | { kind: "fixedBytes"; len: number }
  | { kind: "bytes" | "string"; max: number }
  | { kind: "fixedArray"; item: AbiTypeDescriptor; len: number }
  | { kind: "array"; item: AbiTypeDescriptor; max: number }
  | { kind: "option"; item: AbiTypeDescriptor }
  | { kind: "tuple"; items: AbiTypeDescriptor[] }
  | { kind: "record"; fields: Array<{ name: string; type: AbiTypeDescriptor }> }
  | { kind: "enum"; variants: Array<{ name: string; index: number; type: AbiTypeDescriptor }> }
  | { kind: "result"; ok: AbiTypeDescriptor; err: AbiTypeDescriptor };
export type AbiTypeRef = string | AbiTypeDescriptor;
export type AbiType = { kind: string; max: number | null; descriptor?: AbiTypeDescriptor };
export type AbiField = { name: string; type: AbiTypeRef };
export type AbiAction = {
  name: string;
  selector: string;
  auth: string;
  input: AbiField[];
  executeOutput: AbiTypeRef;
};
export type AbiState = {
  name: string;
  schema: string;
  kind: string;
  keyType: AbiTypeRef;
  valueType: AbiTypeRef;
};
export type AbiQuery = {
  name: string;
  kind: string;
  state: string;
  keyType: AbiTypeRef;
  output: { type: AbiTypeRef; nullable: boolean };
};
export type JamScriptAbi = {
  abiVersion: number;
  languageVersion: string;
  package: { name: string; version: string };
  actions: AbiAction[];
  queries: AbiQuery[];
  types: Record<string, AbiType>;
  state: AbiState[];
};

export type DeploymentDescriptor = {
  genesisHash: string;
  serviceKey: string;
  serviceId: number;
  codeHash: string;
  abiVersion: number;
  abi: JamScriptAbi;
};

export function actionByName(abi: JamScriptAbi, name: string): AbiAction {
  const action = abi.actions.find((value) => value.name === name);
  if (!action) throw new Error("unknown JamScript action: " + name);
  return action;
}

export function queryByName(abi: JamScriptAbi, name: string): AbiQuery {
  const query = abi.queries.find((value) => value.name === name);
  if (!query) throw new Error("unknown JamScript query: " + name);
  return query;
}

export function stateByName(abi: JamScriptAbi, name: string): AbiState {
  const state = abi.state.find((value) => value.name === name);
  if (!state) throw new Error("unknown JamScript state: " + name);
  return state;
}
