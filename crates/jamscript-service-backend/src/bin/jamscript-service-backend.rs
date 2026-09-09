use jamscript_deployment::CurlJsonRpcTransport;
use jamscript_service_backend::{
    ApplicationArtifactLoader, BackendDaemon, BackendRpcHandler, BackendState, DiskBackendStore,
    MiniJamNetworkGateway, PvmArtifactLoader, UnconfiguredWorkGateway,
};
use std::{env, sync::Arc, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind = env::var("JAMSCRIPT_BACKEND_BIND").unwrap_or_else(|_| "0.0.0.0:8090".into());
    let data_root = env::var("JAMSCRIPT_BACKEND_DATA").unwrap_or_else(|_| "./backend-data".into());
    let disk = Arc::new(DiskBackendStore::new(data_root)?);
    let artifact_store = Arc::new(disk.artifact_store()?);
    let loader = Arc::new(PvmArtifactLoader::new(artifact_store.clone()));

    let mut state = BackendState::with_registry(disk.load_registry()?);
    state.replay_recovery_log(disk.recovery_path())?;
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

    let mut handler = BackendRpcHandler::new(state, Arc::new(UnconfiguredWorkGateway))
        .with_pvm_artifact_loader(loader)
        .with_artifact_store(artifact_store)
        .with_persistence(disk.clone());

    if let (Ok(node_rpc), Ok(formal_rpc)) = (
        env::var("JAMSCRIPT_NODE_RPC"),
        env::var("JAMSCRIPT_FORMAL_RPC"),
    ) {
        let network = Arc::new(
            MiniJamNetworkGateway::new(CurlJsonRpcTransport, node_rpc, formal_rpc)
                .with_timeout(Duration::from_secs(30)),
        );
        handler = handler.with_network(network);
    }

    let daemon = BackendDaemon::new(bind, Arc::new(handler));
    daemon.serve().map_err(|error| error.to_string().into())
}
