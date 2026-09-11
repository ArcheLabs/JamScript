use clap::Parser;
use jamscript_deployment::CurlJsonRpcTransport;
use jamscript_service_backend::{
    ApplicationArtifactLoader, BackendDaemon, BackendDatabase, BackendRpcHandler, BackendState,
    DiskArtifactStore, MiniJamNetworkGateway, PvmArtifactLoader, UnconfiguredWorkGateway,
};
use service_runtime_core::{StateRoot, EMPTY_STATE_ROOT_V1};
use std::{env, path::PathBuf, sync::Arc, time::Duration};

#[derive(Debug, Parser)]
#[command(
    name = "jamscript-service-backend",
    about = "JamScript v0.1 managed-state backend"
)]
struct Args {
    #[arg(long)]
    bind: Option<String>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    node_rpc: Option<String>,
    #[arg(long)]
    formal_rpc: Option<String>,
    #[arg(long, default_value = "local")]
    network: String,
    #[arg(long)]
    genesis_hash: Option<String>,
    #[arg(long = "cors-origin", action = clap::ArgAction::Append)]
    cors_origins: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.network != "local" {
        return Err(format!(
            "unsupported backend network `{}`; only local is available",
            args.network
        )
        .into());
    }
    let bind = args
        .bind
        .or_else(|| env::var("JAMSCRIPT_BACKEND_BIND").ok())
        .unwrap_or_else(|| "127.0.0.1:8091".into());
    let node_rpc = args
        .node_rpc
        .or_else(|| env::var("JAMSCRIPT_NODE_RPC").ok());
    let formal_rpc = args
        .formal_rpc
        .or_else(|| env::var("JAMSCRIPT_FORMAL_RPC").ok());
    let node_rpc_for_log = node_rpc.clone();
    let formal_rpc_for_log = formal_rpc.clone();
    let network = match (node_rpc, formal_rpc) {
        (Some(node_rpc), Some(formal_rpc)) => Some(Arc::new(
            MiniJamNetworkGateway::new(CurlJsonRpcTransport, node_rpc, formal_rpc)
                .with_timeout(Duration::from_secs(30)),
        )
            as Arc<dyn jamscript_service_backend::BackendNetwork>),
        (None, None) => None,
        _ => return Err("--node-rpc and --formal-rpc must be supplied together".into()),
    };
    let genesis_hash = if let Some(network) = &network {
        network.genesis_hash()?
    } else if let Some(value) = args
        .genesis_hash
        .or_else(|| env::var("JAMSCRIPT_BACKEND_GENESIS_HASH").ok())
    {
        parse_hash(&value)?
    } else {
        return Err("backend requires a network or --genesis-hash for database binding".into());
    };
    let data_root = args
        .data_dir
        .or_else(|| env::var("JAMSCRIPT_BACKEND_DATA").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            PathBuf::from(".jamscript")
                .join("backend")
                .join(hash_hex(&genesis_hash))
        });
    let db_root = data_root.join("db");
    if !db_root.exists()
        && (data_root.join("registry.json").is_file() || data_root.join("recovery.log").is_file())
    {
        eprintln!(
            "legacy pre-v0.1 backend data detected under {}; archive it or migrate it explicitly",
            data_root.display()
        );
    }
    let database = Arc::new(BackendDatabase::open(db_root, genesis_hash)?);
    let artifact_store = Arc::new(DiskArtifactStore::new(data_root.join("artifacts"))?);
    let loader = Arc::new(PvmArtifactLoader::new(artifact_store.clone()));

    let mut state = BackendState::from_database(database.clone())?;
    let known = state.registry.iter().cloned().collect::<Vec<_>>();
    for record in known {
        match loader.load(&record.application_artifact) {
            Ok(planner) => {
                state.register_planner(record.service_id, planner)?;
            }
            Err(error) => {
                eprintln!(
                    "backend startup: Service {} remains unbound until artifact repair: {:?}",
                    record.service_id, error
                );
            }
        }
    }
    let materialized = state
        .registry
        .iter()
        .filter(|record| !state.is_corrupt(record.service_id))
        .count();
    eprintln!("JamScript backend v{}", env!("CARGO_PKG_VERSION"));
    eprintln!("bind={bind}");
    eprintln!("data_dir={}", data_root.display());
    eprintln!(
        "db_schema={}",
        jamscript_service_backend::BACKEND_DB_SCHEMA_VERSION_V1
    );
    eprintln!("genesis_hash={}", hash_hex(&genesis_hash));
    if let Some(node_rpc) = node_rpc_for_log.as_deref() {
        eprintln!("node_rpc={}", jamscript_deployment::redact_url(node_rpc));
    }
    if let Some(formal_rpc) = formal_rpc_for_log.as_deref() {
        eprintln!(
            "formal_rpc={}",
            jamscript_deployment::redact_url(formal_rpc)
        );
    }
    eprintln!("registered_services={}", state.registry.len());
    eprintln!("materialized_services={materialized}");

    let mut handler = BackendRpcHandler::new(state, Arc::new(UnconfiguredWorkGateway))
        .with_pvm_artifact_loader(loader)
        .with_artifact_store(artifact_store)
        .with_database(database);

    if let Some(network) = network {
        handler = handler.with_network(network);
    }

    let mut daemon = BackendDaemon::new(bind, Arc::new(handler));
    let cors_origins = if args.cors_origins.is_empty() {
        env::var("JAMSCRIPT_BACKEND_CORS_ORIGINS")
            .ok()
            .map(|value| value.split(',').map(str::trim).map(str::to_owned).collect())
    } else {
        Some(args.cors_origins)
    };
    if let Some(origins) = cors_origins {
        daemon = daemon.with_cors_origins(origins)?;
    }
    daemon.serve().map_err(|error| error.to_string().into())
}

fn parse_hash(value: &str) -> Result<StateRoot, Box<dyn std::error::Error>> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() != 64 {
        return Err("genesis hash must be a 32-byte hexadecimal value".into());
    }
    let mut hash = EMPTY_STATE_ROOT_V1;
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        hash[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(hash)
}

fn nibble(value: u8) -> Result<u8, Box<dyn std::error::Error>> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid hexadecimal value".into()),
    }
}

fn hash_hex(bytes: &[u8]) -> String {
    let mut value = String::from("0x");
    for byte in bytes {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}
