#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
command -v docker >/dev/null 2>&1 || {
  echo 'Docker Compose is required for backend publish-boundary checks' >&2
  exit 127
}
command -v jq >/dev/null 2>&1 || {
  echo 'jq is required for backend publish-boundary checks' >&2
  exit 127
}

compose="$(env -u JAMSCRIPT_BACKEND_HOST \
  -u JAMSCRIPT_BACKEND_PORT \
  -u JAMSCRIPT_BACKEND_CORS_ORIGINS \
  docker compose -f "${root}/docker-compose.backend.yml" config --format json)"

jq -e 'any(.services["jamscript-backend"].ports[]?;
  (.published | tostring) == "8090" and .host_ip == "127.0.0.1")' \
  <<<"${compose}" >/dev/null
jq -e 'all(.services["jamscript-backend"].ports[]?;
  .host_ip == "127.0.0.1")' <<<"${compose}" >/dev/null
jq -e '.services["jamscript-backend"].environment.JAMSCRIPT_BACKEND_BIND == "0.0.0.0:8090"' \
  <<<"${compose}" >/dev/null
jq -e '.services["jamscript-backend"].environment.JAMSCRIPT_BACKEND_CORS_ORIGINS == "http://127.0.0.1:5173,http://localhost:5173"' \
  <<<"${compose}" >/dev/null
jq -e '.services["jamscript-backend"].environment.JAMSCRIPT_NODE_RPC == "http://node:9944"' \
  <<<"${compose}" >/dev/null
jq -e '.services["jamscript-backend"].environment.JAMSCRIPT_FORMAL_RPC == "http://formal-rpc:8080"' \
  <<<"${compose}" >/dev/null
jq -e '.services["jamscript-backend"].networks | has("minijam-private-rpc")' \
  <<<"${compose}" >/dev/null
jq -e '.networks["minijam-private-rpc"].external == true and .networks["minijam-private-rpc"].name == "minijam-testnet-chain"' \
  <<<"${compose}" >/dev/null

printf 'BACKEND_HOST_BIND=127.0.0.1:8090\n'
printf 'BACKEND_CONTAINER_BIND=0.0.0.0:8090\n'
printf 'BACKEND_DEFAULT_CORS=LOCALHOST_ALLOWLIST\n'
printf 'BACKEND_PRIVATE_NODE_RPC_NETWORK=minijam-testnet-chain\n'
