export type FinalizedContext = {
  blockHash: string;
  blockNumber: number;
  stateRoot: string;
  slot: number;
  contextType?: "best" | "finalized";
};

export type BestContext = FinalizedContext & { contextType: "best" };

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
  /** Hash of the exact SignedActionV1 bytes submitted by submitAction. */
  actionHash?: string;
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
  | "reorged"
  | "submission_unknown"
  | "failed";

export type SubmitTransactionResult = {
  transactionId: string;
  status: TransactionState;
  packageHash?: string | null;
  itemIndex?: number | null;
  actionIndex?: number | null;
};

export type SubmitActionResult = SubmitTransactionResult & {
  actionHash: string;
  submittedSlot?: number;
  validUntil?: number;
};

export type TransactionStatusResult = {
  transactionId: string;
  status: TransactionState;
  packageHash: string | null;
  itemIndex: number | null;
  actionIndex: number | null;
  executionReceipt: string | null;
  error: string | null;
  errorInfo?: StructuredExecutionError | null;
  bestChainStatus?: "included" | "not_included" | "unknown";
  bestContext?: BestContext;
  finalized?: boolean;
  actionReceipts?: ActionReceipt[];
};

/** Structured guest/runtime failure data returned by the JamScript Backend.
 * Additional fields are intentionally allowed so newer Backends can add
 * diagnostics without forcing an SDK release.
 */
export type StructuredExecutionError = {
  code: string;
  message: string;
  stage?: string;
  submissionState?: "not_submitted";
  serviceId?: number | null;
  codeHash?: string;
  details?: Record<string, unknown>;
  [key: string]: unknown;
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
  stateRoot?: string;
  managedStateRoot?: string;
  serviceKey?: string;
  keyBase64: string;
  valueBase64: string | null;
  proofBase64?: string[];
  finalizedContext?: FinalizedContext;
};

export type BackendCapabilitiesV1 = {
  protocolVersion: number;
  managedStateVersion: number;
  multiService: boolean;
  externalStateWitness: boolean;
  dynamicPvmServices: boolean;
  transactionLifecycleVersion?: number;
  bestChainTracking?: boolean;
  strictFinalizedReceipts?: boolean;
  durableTransactionLookup?: boolean;
};

export class RpcError extends Error {
  readonly structuredError?: StructuredExecutionError;

  constructor(
    message: string,
    readonly code: number,
    readonly data?: unknown,
  ) {
    super(message);
    this.structuredError = isStructuredExecutionError(data)
      ? data
      : isStructuredExecutionError((data as { cause?: unknown } | null)?.cause)
        ? {
            ...((data as { cause: StructuredExecutionError }).cause),
            submissionState: "not_submitted",
          }
        : undefined;
  }
}

function isStructuredExecutionError(value: unknown): value is StructuredExecutionError {
  return typeof value === "object"
    && value !== null
    && typeof (value as { code?: unknown }).code === "string"
    && typeof (value as { message?: unknown }).message === "string";
}

export interface RpcTransport {
  call<T>(method: string, params?: unknown): Promise<T>;
}

const FORMAL_WORK_METHODS = new Set([
  "minijam_submitWorkV1",
  "minijam_getWorkStatusV1",
]);

const STATE_PROVIDER_METHODS = new Set([
  "jamscript_getStateV1",
  "jamscript_getStateProofV1",
  "minijam_getManagedStateV1",
]);

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
  bestContext(): Promise<BestContext>;
  genesisHash(): Promise<string>;
  serviceStorageAt(blockHash: string, serviceId: number, key: string): Promise<string | null>;
  managedStateAt(
    serviceId: number,
    stateRoot: string,
    keyBase64: string,
    context?: FinalizedContext,
  ): Promise<ManagedStateResult>;
  submitWork(request: SubmitWorkRequest): Promise<SubmitWorkResult>;
  workStatus(packageHash: string, serviceId?: number): Promise<WorkStatusResult>;
  submitTransaction(request: SubmitTransactionRequest): Promise<SubmitTransactionResult>;
  transactionStatus(transactionId: string): Promise<TransactionStatusResult>;
  capabilities(): Promise<BackendCapabilitiesV1>;
};

export function asWorkRpc(transport: RpcTransport): WorkRpc {
  return {
    call: transport.call.bind(transport),
    finalizedContext: async () => ({
      ...await transport.call<FinalizedContext>("minijam_getFinalizedContext"),
      contextType: "finalized",
    }),
    bestContext: async () => ({
      ...await transport.call<FinalizedContext>("minijam_getBestContext"),
      contextType: "best",
    }),
    genesisHash: () => transport.call("chain_getBlockHash", [0]),
    serviceStorageAt: (blockHash, serviceId, key) =>
      transport.call("minijam_getServiceStorageAt", [blockHash, serviceId, key]),
    managedStateAt: (serviceId, stateRoot, keyBase64, context) =>
      transport.call("minijam_getManagedStateV1", {
        serviceId,
        stateRoot,
        keyBase64,
        ...(context ? { context } : {}),
      }),
    submitWork: (request) => transport.call("minijam_submitWorkV1", request),
    workStatus: (packageHash, serviceId) =>
      transport.call(
        "minijam_getWorkStatusV1",
        serviceId === undefined ? { packageHash } : { packageHash, serviceId },
      ),
    submitTransaction: (request) =>
      transport.call("jamscript_submitTransactionV1", request),
    transactionStatus: async (transactionId) => {
      const result = await transport.call<TransactionStatusResult & { receipt?: string | null }>(
        "jamscript_getTransactionStatusV1",
        { transactionId },
      );
      return {
        ...result,
        packageHash: result.packageHash ?? null,
        itemIndex: result.itemIndex ?? null,
        actionIndex: result.actionIndex ?? null,
        executionReceipt: result.executionReceipt ?? result.receipt ?? null,
        error: result.error ?? null,
      };
    },
    capabilities: () => transport.call<BackendCapabilitiesV1>("jamscript_getCapabilitiesV1"),
  };
}
