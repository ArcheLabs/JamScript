use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use service_runtime_core::{
    ManagedStateCommitmentV1, RuntimeRefineOutputV2,
    ServiceKeyV1, StateRecoveryV1, EMPTY_STATE_ROOT_V1, MANAGED_STATE_COMMITMENT_KEY_V1,
};
use service_runtime_host::{
    AuthenticatedWorkBuilder, BuiltManagedWorkV1, BuiltManagedWorkV2, FinalizedContextV1,
    FinalizedManagedStateSource, FullStateProvider, ManagedStateWorkBuilder,
    MaterializedServiceStateProvider, ServiceStateProvider,
};

const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;

include!(env!("JAMSCRIPT_BUILDER_APPLICATION_RS"));

#[derive(Clone)]
struct Config {
    bind: String,
    node_url: String,
    formal_url: String,
    service_id: u32,
    service_key: ServiceKeyV1,
    code_hash: [u8; 32],
    test_methods: bool,
    provider_store: Option<PathBuf>,
}

#[derive(Default)]
struct AdapterState {
    provider: FullStateProvider,
    pending: BTreeMap<String, RuntimeRefineOutputV2>,
    predictions: BTreeMap<String, RuntimeRefineOutputV2>,
    query_fault: Option<String>,
}

#[derive(Clone)]
struct Adapter {
    config: Config,
    state: Arc<Mutex<AdapterState>>,
}

enum BuiltWork {
    V1(BuiltManagedWorkV1),
    V2(BuiltManagedWorkV2),
}

impl BuiltWork {
    fn predicted_output(&self) -> &RuntimeRefineOutputV2 {
        match self {
            Self::V1(work) => &work.predicted_output,
            Self::V2(work) => &work.predicted_output,
        }
    }

    fn encode_refine_input(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::V1(work) => work
                .refine_input
                .encode()
                .map_err(|error| format!("encode V1 refine input: {error:?}")),
            Self::V2(work) => work
                .refine_input
                .encode()
                .map_err(|error| format!("encode V2 refine input: {error:?}")),
        }
    }

    fn tampered_verifier_rejects(&mut self, application: &GeneratedApplication) -> Result<bool, String> {
        match self {
            Self::V1(work) => {
                let node = work
                    .refine_input
                    .managed_state
                    .storage_proof
                    .first_mut()
                    .ok_or_else(|| "V1 witness is empty".to_owned())?;
                let byte = node
                    .first_mut()
                    .ok_or_else(|| "V1 witness node is empty".to_owned())?;
                *byte ^= 1;
                Ok(service_runtime_guest::refine(application, &work.refine_input).is_err())
            }
            Self::V2(work) => {
                let node = work
                    .refine_input
                    .managed_state
                    .storage_proof
                    .first_mut()
                    .ok_or_else(|| "V2 witness is empty".to_owned())?;
                let byte = node
                    .first_mut()
                    .ok_or_else(|| "V2 witness node is empty".to_owned())?;
                *byte ^= 1;
                Ok(service_runtime_guest::refine_input_v2(
                    application,
                    &work.refine_input,
                )
                .is_err())
            }
        }
    }
}

fn build_work(
    source: &mut NodeSource<'_>,
    provider: &FullStateProvider,
    service: ServiceKeyV1,
    application: &GeneratedApplication,
    action: Vec<u8>,
) -> Result<BuiltWork, RpcFailure> {
    match JAMSCRIPT_RUNTIME_REFINE_INPUT_VERSION {
        1 => ManagedStateWorkBuilder::new(source, provider)
            .build_one(service, application, action)
            .map(BuiltWork::V1)
            .map_err(|error| RpcFailure::builder(format!("{error:?}"))),
        2 => AuthenticatedWorkBuilder::new(source, provider)
            .build_actions(service, application, vec![action])
            .map(BuiltWork::V2)
            .map_err(|error| RpcFailure::builder(format!("{error:?}"))),
        version => Err(RpcFailure::builder(format!(
            "unsupported generated runtime refine input version {version}"
        ))),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextParams {
    block_hash: String,
    state_root: String,
    slot: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitParams {
    context: ContextParams,
    service_id: u32,
    service_code_hash: String,
    payload_base64: String,
    #[serde(default)]
    extrinsics_base64: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusParams {
    package_hash: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryParams {
    service_id: u32,
    state_root: String,
    key_base64: String,
}

#[derive(Deserialize)]
struct RpcRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeContext {
    block_hash: String,
    block_number: u32,
    state_root: String,
    slot: u32,
}

struct NodeSource<'a> {
    config: &'a Config,
    context: NodeContext,
}

impl FinalizedManagedStateSource for NodeSource<'_> {
    type Error = String;

    fn finalized_context(&mut self) -> Result<FinalizedContextV1, Self::Error> {
        Ok(FinalizedContextV1 {
            block_hash: parse_hash(&self.context.block_hash)?,
            state_root: parse_hash(&self.context.state_root)?,
            slot: u64::from(self.context.slot),
        })
    }

    fn service_storage_at(
        &mut self,
        context: &FinalizedContextV1,
        service: ServiceKeyV1,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        if service != self.config.service_key || key != MANAGED_STATE_COMMITMENT_KEY_V1 {
            return Err("unexpected managed-state identity".into());
        }
        match rpc_call(
            &self.config.node_url,
            "minijam_getServiceStorageAt",
            json!([hex(&context.block_hash), self.config.service_id, hex(key)]),
        )? {
            Value::Null => Ok(None),
            Value::String(encoded) => Ok(Some(decode_state_value(&parse_hex(&encoded)?)?)),
            _ => Err("invalid Service storage response".into()),
        }
    }
}

impl Adapter {
    fn handle_rpc(&self, request: RpcRequest) -> Value {
        let id = request.id;
        let result = match request.method.as_str() {
            "minijam_submitWorkV1" => {
                parse_params(request.params).and_then(|p| self.submit(p, false))
            }
            "jamscript_testSubmitTamperedWitnessV1" => self
                .require_test()
                .and_then(|_| parse_params(request.params))
                .and_then(|p| self.submit(p, true)),
            "jamscript_testGetPredictionV1" => self
                .require_test()
                .and_then(|_| parse_params::<StatusParams>(request.params))
                .and_then(|p| self.prediction(&p.package_hash)),
            "jamscript_testForgetProviderV1" => self.require_test().map(|_| {
                self.state.lock().expect("adapter state lock").provider =
                    FullStateProvider::default();
                json!(true)
            }),
            "jamscript_testCorruptNextQueryV1" => self
                .require_test()
                .and_then(|_| {
                    request
                        .params
                        .get("mode")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| RpcFailure::invalid("mode is required"))
                })
                .map(|mode| {
                    self.state.lock().expect("adapter state lock").query_fault = Some(mode);
                    json!(true)
                }),
            "minijam_getWorkStatusV1" => parse_params(request.params).and_then(|p| self.status(p)),
            "minijam_getManagedStateV1" => parse_params(request.params).and_then(|p| self.query(p)),
            _ => Err(RpcFailure::new(-32601, "method not found", None)),
        };
        match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => json!({
                "jsonrpc":"2.0","id":id,
                "error":{"code":error.code,"message":error.message,"data":error.data}
            }),
        }
    }

    fn submit(&self, mut request: SubmitParams, tamper_witness: bool) -> RpcResult {
        self.validate_request(&request)?;
        let finalized = self.node_context().map_err(RpcFailure::chain)?;
        if request.context.block_hash.to_lowercase() != finalized.block_hash.to_lowercase()
            || request.context.state_root.to_lowercase() != finalized.state_root.to_lowercase()
            || request.context.slot != finalized.slot
        {
            return Err(RpcFailure::new(
                -32010,
                "stale finalized context",
                Some(serde_json::to_value(finalized).expect("context serializes")),
            ));
        }
        let action = STANDARD
            .decode(&request.payload_base64)
            .map_err(|error| RpcFailure::invalid(error.to_string()))?;
        let application = GeneratedApplication;
        let mut state = self.state.lock().expect("adapter state lock");
        let mut source = NodeSource {
            config: &self.config,
            context: finalized.clone(),
        };
        let mut built = build_work(
            &mut source,
            &state.provider,
            self.config.service_key,
            &application,
            action,
        )?;
        let predicted_output = built.predicted_output().clone();
        if self.config.test_methods {
            let recovery = StateRecoveryV1::decode(&predicted_output.recovery_payload)
                .map_err(|error| RpcFailure::builder(format!("recovery decode: {error:?}")))?;
            eprintln!(
                "predicted transition parent={} new={} receipts={:?} changes={:?}",
                hex(&predicted_output.parent_root),
                hex(&predicted_output.new_root),
                predicted_output.receipts,
                recovery.diff.changes
            );
        }
        if predicted_output
            .transition_valid_until
            .is_some_and(|valid_until| u64::from(finalized.slot) > valid_until)
        {
            return Err(RpcFailure::new(
                -32032,
                "managed-state preflight rejected expired transition",
                Some(json!({"finalizedSlot": finalized.slot})),
            ));
        }
        if tamper_witness {
            if !built
                .tampered_verifier_rejects(&application)
                .map_err(RpcFailure::builder)?
            {
                return Err(RpcFailure::builder(
                    "tampered witness unexpectedly passed verifier execution",
                ));
            }
            return Err(RpcFailure::new(
                -32033,
                "managed-state preflight rejected tampered witness",
                None,
            ));
        }
        request.payload_base64 = STANDARD.encode(
            built
                .encode_refine_input()
                .map_err(RpcFailure::builder)?,
        );
        request.context = ContextParams {
            block_hash: finalized.block_hash,
            state_root: finalized.state_root,
            slot: finalized.slot,
        };
        let result = rpc_call(
            &self.config.formal_url,
            "minijam_submitWorkV1",
            serde_json::to_value(request).expect("request serializes"),
        )
        .map_err(RpcFailure::downstream)?;
        let package_hash = result
            .get("packageHash")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcFailure::chain("formal RPC omitted packageHash"))?
            .to_owned();
        if !tamper_witness {
            state
                .pending
                .insert(package_hash.to_lowercase(), predicted_output.clone());
            state
                .predictions
                .insert(package_hash.to_lowercase(), predicted_output);
        }
        Ok(result)
    }

    fn status(&self, request: StatusParams) -> RpcResult {
        let mut result = rpc_call(
            &self.config.formal_url,
            "minijam_getWorkStatusV1",
            json!({"packageHash":request.package_hash}),
        )
        .map_err(RpcFailure::downstream)?;
        if result.get("status").and_then(Value::as_str) == Some("imported") {
            self.materialize(&request.package_hash)?;
            if let Some(output) = self
                .state
                .lock()
                .expect("adapter state lock")
                .predictions
                .get(&request.package_hash.to_lowercase())
                .cloned()
            {
                if let Some(object) = result.as_object_mut() {
                    object.insert("actionReceipts".into(), action_receipts(&output));
                }
            }
        }
        Ok(result)
    }

    fn materialize(&self, package_hash: &str) -> Result<(), RpcFailure> {
        let output = self
            .state
            .lock()
            .expect("adapter state lock")
            .pending
            .get(&package_hash.to_lowercase())
            .cloned();
        let Some(output) = output else { return Ok(()) };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let finalized = self.node_context().map_err(RpcFailure::chain)?;
            if output
                .transition_valid_until
                .is_some_and(|valid_until| u64::from(finalized.slot) > valid_until)
            {
                self.state
                    .lock()
                    .expect("adapter state lock")
                    .pending
                    .remove(&package_hash.to_lowercase());
                return Ok(());
            }
            let mut source = NodeSource {
                config: &self.config,
                context: finalized,
            };
            let context = source.finalized_context().map_err(RpcFailure::chain)?;
            let commitment = source
                .service_storage_at(
                    &context,
                    self.config.service_key,
                    MANAGED_STATE_COMMITMENT_KEY_V1,
                )
                .map_err(RpcFailure::chain)?;
            let root = commitment
                .as_deref()
                .map(ManagedStateCommitmentV1::decode)
                .transpose()
                .map_err(|_| RpcFailure::chain("invalid finalized commitment"))?
                .map_or(EMPTY_STATE_ROOT_V1, |value| value.root);
            if root == output.new_root {
                let mut state = self.state.lock().expect("adapter state lock");
                state
                    .provider
                    .apply_recovery(self.config.service_key, &output)
                    .map_err(|error| RpcFailure::builder(format!("recovery: {error:?}")))?;
                if let Some(path) = &self.config.provider_store {
                    append_recovery(path, &output).map_err(RpcFailure::builder)?;
                }
                state.pending.remove(&package_hash.to_lowercase());
                return Ok(());
            }
            if root != output.parent_root || Instant::now() >= deadline {
                return Err(RpcFailure::builder(format!(
                    "finalized commitment rejected predicted transition: parent={} predicted={} finalized={} receipts={:?}",
                    hex(&output.parent_root),
                    hex(&output.new_root),
                    hex(&root),
                    output.receipts,
                )));
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    fn query(&self, request: QueryParams) -> RpcResult {
        if request.service_id != self.config.service_id {
            return Err(RpcFailure::new(-32011, "service not found", None));
        }
        let root = parse_hash(&request.state_root).map_err(RpcFailure::invalid)?;
        let key = STANDARD
            .decode(&request.key_base64)
            .map_err(|error| RpcFailure::invalid(error.to_string()))?;
        let mut state = self.state.lock().expect("adapter state lock");
        let mut response = state
            .provider
            .get(self.config.service_key, root, &key)
            .map_err(|error| RpcFailure::builder(format!("{error:?}")))?;
        match state.query_fault.take().as_deref() {
            Some("value") => match response.value.as_mut() {
                Some(value) if !value.is_empty() => value[0] ^= 1,
                _ => response.value = Some(vec![1]),
            },
            Some("proof") => {
                if let Some(node) = response.proof.first_mut() {
                    node[0] ^= 1;
                }
            }
            _ => {}
        }
        Ok(json!({
            "serviceId":request.service_id,
            "stateRoot":hex(&response.state_root),
            "keyBase64":request.key_base64,
            "valueBase64":response.value.map(|value| STANDARD.encode(value)),
            "proofBase64":response.proof.into_iter().map(|node| STANDARD.encode(node)).collect::<Vec<_>>()
        }))
    }

    fn validate_request(&self, request: &SubmitParams) -> Result<(), RpcFailure> {
        if request.service_id != self.config.service_id {
            return Err(RpcFailure::new(-32011, "service not found", None));
        }
        if parse_hash(&request.service_code_hash).map_err(RpcFailure::invalid)?
            != self.config.code_hash
        {
            return Err(RpcFailure::new(-32012, "service code hash mismatch", None));
        }
        Ok(())
    }

    fn require_test(&self) -> Result<(), RpcFailure> {
        if self.config.test_methods {
            Ok(())
        } else {
            Err(RpcFailure::new(-32601, "method not found", None))
        }
    }

    fn prediction(&self, package_hash: &str) -> RpcResult {
        let output = self
            .state
            .lock()
            .expect("adapter state lock")
            .predictions
            .get(&package_hash.to_lowercase())
            .cloned()
            .ok_or_else(|| RpcFailure::new(-32013, "prediction not found", None))?;
        Ok(json!({
            "parentRoot":hex(&output.parent_root),
            "newRoot":hex(&output.new_root),
            "validUntil":output.transition_valid_until,
            "actionReceipts":action_receipts(&output)
        }))
    }

    fn node_context(&self) -> Result<NodeContext, String> {
        serde_json::from_value(rpc_call(
            &self.config.node_url,
            "minijam_getFinalizedContext",
            json!([]),
        )?)
        .map_err(|error| error.to_string())
    }
}

type RpcResult = Result<Value, RpcFailure>;

#[derive(Debug)]
struct RpcFailure {
    code: i32,
    message: String,
    data: Option<Value>,
}

impl RpcFailure {
    fn new(code: i32, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            code,
            message: message.into(),
            data,
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new(-32602, message, None)
    }
    fn chain(message: impl Into<String>) -> Self {
        Self::new(-32021, message, None)
    }
    fn downstream(message: String) -> Self {
        serde_json::from_str::<Value>(&message)
            .ok()
            .and_then(|error| {
                Some(Self::new(
                    i32::try_from(error.get("code")?.as_i64()?).ok()?,
                    error.get("message")?.as_str()?.to_owned(),
                    error.get("data").cloned(),
                ))
            })
            .unwrap_or_else(|| Self::chain(message))
    }
    fn builder(message: impl Into<String>) -> Self {
        Self::new(-32030, message, None)
    }
}

fn main() -> Result<(), String> {
    if env!("JAMSCRIPT_BUILDER_ARTIFACT_CONFIGURED") != "1" {
        return Err(
            "generated Builder application is required; compile with JAMSCRIPT_BUILDER_APPLICATION_RS"
                .into(),
        );
    }
    let config = Config {
        bind: env::var("JAMSCRIPT_ADAPTER_BIND").unwrap_or_else(|_| "127.0.0.1:8091".into()),
        node_url: env::var("MINIJAM_NODE_RPC").unwrap_or_else(|_| "http://127.0.0.1:9944".into()),
        formal_url: env::var("MINIJAM_FORMAL_RPC_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8090".into()),
        service_id: env_parse("JAMSCRIPT_E2E_SERVICE_ID")?,
        service_key: ServiceKeyV1::new(parse_hash(&required_env("JAMSCRIPT_E2E_SERVICE_KEY")?)?),
        code_hash: parse_hash(&required_env("JAMSCRIPT_E2E_CODE_HASH")?)?,
        test_methods: env::var("JAMSCRIPT_E2E_TEST_METHODS").as_deref() == Ok("true"),
        provider_store: env::var_os("JAMSCRIPT_PROVIDER_STORE").map(PathBuf::from),
    };
    let provider = load_provider(config.provider_store.as_deref(), config.service_key)?;
    let listener = TcpListener::bind(&config.bind).map_err(|error| error.to_string())?;
    let adapter = Adapter {
        config,
        state: Arc::new(Mutex::new(AdapterState {
            provider,
            ..Default::default()
        })),
    };
    eprintln!(
        "JamScript managed-state adapter listening on {}",
        adapter.config.bind
    );
    for stream in listener.incoming() {
        let adapter = adapter.clone();
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    let _ = handle_http(stream, &adapter);
                });
            }
            Err(error) => eprintln!("adapter accept error: {error}"),
        }
    }
    Ok(())
}

fn load_provider(path: Option<&Path>, service: ServiceKeyV1) -> Result<FullStateProvider, String> {
    let Some(path) = path else {
        return Ok(FullStateProvider::default());
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FullStateProvider::default())
        }
        Err(error) => return Err(format!("reading Provider recovery log: {error}")),
    };
    let mut provider = FullStateProvider::default();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let length_bytes = bytes
            .get(offset..offset + 4)
            .ok_or("truncated Provider recovery log length")?;
        let length = u32::from_le_bytes(length_bytes.try_into().expect("four bytes")) as usize;
        offset += 4;
        let encoded = bytes
            .get(offset..offset + length)
            .ok_or("truncated Provider recovery log entry")?;
        offset += length;
        let output = RuntimeRefineOutputV2::decode(encoded)
            .map_err(|_| "invalid Provider recovery log entry")?;
        provider
            .apply_recovery(service, &output)
            .map_err(|error| format!("invalid Provider recovery chain: {error:?}"))?;
    }
    Ok(provider)
}

fn append_recovery(path: &Path, output: &RuntimeRefineOutputV2) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let encoded = output
        .encode()
        .map_err(|error| format!("encoding Provider recovery: {error:?}"))?;
    let length = u32::try_from(encoded.len()).map_err(|_| "Provider recovery entry too large")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(&length.to_le_bytes())
        .and_then(|_| file.write_all(&encoded))
        .and_then(|_| file.sync_data())
        .map_err(|error| error.to_string())
}

fn action_receipts(output: &RuntimeRefineOutputV2) -> Value {
    Value::Array(
        output
            .receipts
            .iter()
            .map(|receipt| {
                json!({
                    "actionHash": hex(&receipt.action_hash),
                    "status": match receipt.status {
                        service_runtime_core::ActionStatusV1::Applied => "applied",
                        service_runtime_core::ActionStatusV1::Failed => "failed",
                        service_runtime_core::ActionStatusV1::Rejected => "rejected",
                    },
                    "errorCode": receipt.error_code,
                })
            })
            .collect(),
    )
}

fn handle_http(mut stream: TcpStream, adapter: &Adapter) -> Result<(), String> {
    let request = read_http(&mut stream)?;
    if request.method == "GET" && request.path == "/health/ready" {
        return write_http(&mut stream, 200, &json!({"status":"ready"}).to_string());
    }
    if request.method != "POST" || request.path != "/" {
        return write_http(&mut stream, 404, "not found");
    }
    let rpc: RpcRequest =
        serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;
    write_http(&mut stream, 200, &adapter.handle_rpc(rpc).to_string())
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let header_end;
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("truncated HTTP request".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_HTTP_BYTES {
            return Err("HTTP request too large".into());
        }
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|error| error.to_string())?;
    let mut lines = header.split("\r\n");
    let mut start = lines
        .next()
        .ok_or("missing HTTP request line")?
        .split_whitespace();
    let method = start.next().ok_or("missing HTTP method")?.to_owned();
    let path = start.next().ok_or("missing HTTP path")?.to_owned();
    let length = lines
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    if length > MAX_HTTP_BYTES {
        return Err("HTTP request too large".into());
    }
    while bytes.len() < header_end + length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("truncated HTTP body".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        body: bytes[header_end..header_end + length].to_vec(),
    })
}

fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    write!(stream, "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).map_err(|error| error.to_string())
}

fn rpc_call(url: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string();
    let (host, port, path) = parse_http_url(url)?;
    let mut stream =
        TcpStream::connect((host.as_str(), port)).map_err(|error| error.to_string())?;
    write!(stream, "POST {path} HTTP/1.1\r\nhost: {host}:{port}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).map_err(|error| error.to_string())?;
    let response = read_to_end(&mut stream)?;
    let body_at = find_bytes(&response, b"\r\n\r\n").ok_or("invalid HTTP response")? + 4;
    let value: Value =
        serde_json::from_slice(&response[body_at..]).map_err(|error| error.to_string())?;
    if let Some(error) = value.get("error") {
        return Err(error.to_string());
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "RPC response omitted result".into())
}

fn parse_http_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("only http:// URLs are supported")?;
    let (authority, path) = rest.split_once('/').map_or((rest, "/"), |(authority, _)| {
        (authority, &rest[authority.len()..])
    });
    let (host, port) = authority
        .split_once(':')
        .ok_or("RPC URL must include a port")?;
    Ok((
        host.to_owned(),
        port.parse().map_err(|_| "invalid RPC port")?,
        path.to_owned(),
    ))
}

fn read_to_end(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    stream
        .read_to_end(&mut output)
        .map_err(|error| error.to_string())?;
    Ok(output)
}

fn decode_state_value(encoded: &[u8]) -> Result<Vec<u8>, String> {
    let (length, prefix) = decode_compact(encoded)?;
    if prefix + length != encoded.len() {
        return Err("invalid StateValue length".into());
    }
    Ok(encoded[prefix..].to_vec())
}

fn decode_compact(input: &[u8]) -> Result<(usize, usize), String> {
    let first = *input.first().ok_or("empty compact value")?;
    match first & 3 {
        0 => Ok(((first >> 2) as usize, 1)),
        1 if input.len() >= 2 => Ok(((u16::from_le_bytes([first, input[1]]) >> 2) as usize, 2)),
        2 if input.len() >= 4 => Ok((
            (u32::from_le_bytes(input[..4].try_into().expect("four bytes")) >> 2) as usize,
            4,
        )),
        _ => Err("unsupported compact value".into()),
    }
}

fn parse_hash(value: &str) -> Result<[u8; 32], String> {
    parse_hex(value)?
        .try_into()
        .map_err(|_| "expected 32-byte hex".into())
}

fn parse_hex(value: &str) -> Result<Vec<u8>, String> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if !value.len().is_multiple_of(2) {
        return Err("invalid hex length".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

fn nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid hex".into()),
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, RpcFailure> {
    serde_json::from_value(value).map_err(|error| RpcFailure::invalid(error.to_string()))
}

fn hex(bytes: &[u8]) -> String {
    format!(
        "0x{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn required_env(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("{name} is required"))
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Result<T, String> {
    required_env(name)?
        .parse()
        .map_err(|_| format!("invalid {name}"))
}
