export type FinalizedContext = {
  blockHash: string;
  blockNumber: number;
  stateRoot: string;
  slot: number;
};

export type SubmitWorkRequest = {
  context: { blockHash: string; stateRoot: string; slot: number };
  serviceId: number;
  serviceCodeHash: string;
  payloadBase64: string;
  extrinsicsBase64: string[];
};

export type SubmitWorkResult = {
  packageHash: string;
  submissionHash: string;
  context: FinalizedContext;
  /** Hash of the exact SignedActionV2 bytes submitted by submitAction. */
  actionHash?: string;
};

export type WorkStatus =
  | "insufficient_workers"
  | "awaiting_candidate"
  | "voting"
  | "accepted"
  | "imported"
  | "failed";

export type WorkStatusResult = {
  packageHash: string;
  workId: number | null;
  status: WorkStatus;
  executionReceipt: string | null;
  actionReceipts?: ActionReceipt[];
  context: FinalizedContext;
};

export type ActionReceipt = {
  actionHash: string;
  status: "applied" | "failed" | "rejected";
  errorCode: number | null;
};

export type ManagedStateResult = {
  serviceId: number;
  stateRoot: string;
  keyBase64: string;
  valueBase64: string | null;
  proofBase64: string[];
};

export class RpcError extends Error {
  constructor(
    message: string,
    readonly code: number,
    readonly data?: unknown,
  ) {
    super(message);
  }
}

export interface RpcTransport {
  call<T>(method: string, params?: unknown): Promise<T>;
}

const FORMAL_WORK_METHODS = new Set([
  "minijam_submitWorkV1",
  "minijam_getWorkStatusV1",
]);

const STATE_PROVIDER_METHODS = new Set(["minijam_getManagedStateV1"]);

export class SplitRpcTransport implements RpcTransport {
  constructor(
    private readonly node: RpcTransport,
    private readonly work: RpcTransport,
    private readonly state: RpcTransport = node,
  ) {}

  call<T>(method: string, params?: unknown): Promise<T> {
    const transport = FORMAL_WORK_METHODS.has(method)
      ? this.work
      : STATE_PROVIDER_METHODS.has(method)
        ? this.state
        : this.node;
    return transport.call<T>(method, params);
  }
}

export class FetchRpcTransport implements RpcTransport {
  private nextId = 1;

  constructor(private readonly endpoint: string, private readonly fetchImpl = fetch) {}

  async call<T>(method: string, params: unknown = []): Promise<T> {
    const response = await this.fetchImpl(this.endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: this.nextId++, method, params }),
    });
    if (!response.ok) throw new RpcError("RPC HTTP " + response.status, response.status);
    const body = (await response.json()) as {
      result?: T;
      error?: { code: number; message: string; data?: unknown };
    };
    if (body.error) throw new RpcError(body.error.message, body.error.code, body.error.data);
    if (!("result" in body)) throw new RpcError("RPC response has no result", -32000);
    return body.result as T;
  }
}

export type WorkRpc = RpcTransport & {
  finalizedContext(): Promise<FinalizedContext>;
  genesisHash(): Promise<string>;
  serviceStorageAt(blockHash: string, serviceId: number, key: string): Promise<string | null>;
  managedStateAt(
    serviceId: number,
    stateRoot: string,
    keyBase64: string,
  ): Promise<ManagedStateResult>;
  submitWork(request: SubmitWorkRequest): Promise<SubmitWorkResult>;
  workStatus(packageHash: string): Promise<WorkStatusResult>;
};

export function asWorkRpc(transport: RpcTransport): WorkRpc {
  return {
    call: transport.call.bind(transport),
    finalizedContext: () => transport.call("minijam_getFinalizedContext"),
    genesisHash: () => transport.call("chain_getBlockHash", [0]),
    serviceStorageAt: (blockHash, serviceId, key) =>
      transport.call("minijam_getServiceStorageAt", [blockHash, serviceId, key]),
    managedStateAt: (serviceId, stateRoot, keyBase64) =>
      transport.call("minijam_getManagedStateV1", { serviceId, stateRoot, keyBase64 }),
    submitWork: (request) => transport.call("minijam_submitWorkV1", request),
    workStatus: (packageHash) =>
      transport.call("minijam_getWorkStatusV1", { packageHash }),
  };
}
