export type AbiType = { kind: string; max: number | null };
export type AbiField = { name: string; type: string };
export type AbiAction = {
  name: string;
  selector: string;
  auth: string;
  input: AbiField[];
  executeOutput: string;
};
export type AbiState = {
  name: string;
  schema: string;
  kind: string;
  keyType: string;
  valueType: string;
};
export type AbiQuery = {
  name: string;
  kind: string;
  state: string;
  keyType: string;
  output: { type: string; nullable: boolean };
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
