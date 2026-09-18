#!/usr/bin/env bash
set -euo pipefail

IMAGE="${1:?usage: backend-container-smoke.sh IMAGE [PORT] [VOLUME] [CONTAINER]}"
PORT="${2:-18090}"
VOLUME="${3:-jamscript-backend-smoke-${GITHUB_RUN_ID:-$$}}"
CONTAINER="${4:-jamscript-backend-smoke-${GITHUB_RUN_ID:-$$}}"
GENESIS_HASH="0x$(printf '00%.0s' {1..32})"

cleanup() {
  docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
  docker volume rm "${VOLUME}" >/dev/null 2>&1 || true
}
diagnose() {
  docker inspect "${CONTAINER}" || true
  docker logs "${CONTAINER}" || true
}
trap cleanup EXIT

docker volume create "${VOLUME}" >/dev/null

wait_for_ready() {
  local health=0
  local running
  for _ in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
      health=1
      break
    fi
    running="$(docker inspect -f '{{.State.Running}}' "${CONTAINER}" 2>/dev/null || echo false)"
    if [[ "${running}" != true ]]; then
      echo "BACKEND_CONTAINER_EXITED=YES" >&2
      diagnose >&2
      return 1
    fi
    sleep 1
  done
  if [[ "${health}" != 1 ]]; then
    echo "BACKEND_HEALTH_TIMEOUT=YES" >&2
    diagnose >&2
    return 1
  fi
  curl -fsS "http://127.0.0.1:${PORT}/readinessz" >/dev/null || {
    echo "BACKEND_READINESS=FAIL" >&2
    diagnose >&2
    return 1
  }
}

start_container() {
  docker run -d --name "${CONTAINER}" -p "${PORT}:8090" \
    -e JAMSCRIPT_BACKEND_CORS_ORIGINS='*' \
    -v "${VOLUME}:/var/lib/jamscript" \
    "${IMAGE}" --bind 0.0.0.0:8090 --genesis-hash "${GENESIS_HASH}" >/dev/null
}

start_container
wait_for_ready
echo "BACKEND_HEALTH=PASS"
echo "BACKEND_READINESS=PASS"
docker rm -f "${CONTAINER}" >/dev/null
start_container
wait_for_ready
echo "BACKEND_VOLUME_RESTART=PASS"
