export type FinalizedContext = {
  blockHash: string;
  blockNumber: number;
  stateRoot: string;
  slot: number;
};

export type SubmitTransactionRequest = {
  serviceId: number;
  serviceCodeHash: string;
  payloadBase64: string;
  extrinsicsBase64: string[];
};

export type TransactionState =
  | "queued"
  | "packaged"
  | "refining"
  | "reported"
  | "imported"
  | "failed";

export type SubmitTransactionResult = {
  transactionId: string;
  status: TransactionState;
  packageHash?: string | null;
  itemIndex?: number | null;
};

export type SubmitActionResult = SubmitTransactionResult & {
  /** Hash of the exact SignedActionV1 bytes submitted by submitAction. */
  actionHash: string;
};

export type TransactionStatusResult = {
  transactionId: string;
  status: TransactionState;
  packageHash: string | null;
  itemIndex: number | null;
  executionReceipt: string | null;
  error: string | null;
  /** Optional application-level receipts exposed by a compatible gateway. */
  actionReceipts?: ActionReceipt[];
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
  "minijam_submitTransactionV1",
  "minijam_getTransactionStatusV1",
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
  submitTransaction(request: SubmitTransactionRequest): Promise<SubmitTransactionResult>;
  transactionStatus(transactionId: string): Promise<TransactionStatusResult>;
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
    submitTransaction: (request) => transport.call("minijam_submitTransactionV1", request),
    transactionStatus: async (transactionId) => {
      const result = await transport.call<TransactionStatusResult & { receipt?: string | null }>(
        "minijam_getTransactionStatusV1",
        { transactionId },
      );
      return {
        ...result,
        packageHash: result.packageHash ?? null,
        itemIndex: result.itemIndex ?? null,
        executionReceipt: result.executionReceipt ?? result.receipt ?? null,
        error: result.error ?? null,
      };
    },
  };
}
