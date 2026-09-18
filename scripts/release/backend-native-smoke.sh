#!/usr/bin/env bash
set -euo pipefail

BINARY="${1:?usage: backend-native-smoke.sh BACKEND_BINARY [PORT]}"
PORT="${2:-18091}"
test -x "${BINARY}"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/jamscript-backend-native-smoke.XXXXXX")"
pid=""
cleanup() {
  if [[ -n "${pid}" ]]; then
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

"${BINARY}" \
  --bind "127.0.0.1:${PORT}" \
  --data-dir "${work_dir}/data" \
  --genesis-hash "0x$(printf '00%.0s' {1..32})" \
  >"${work_dir}/stdout.log" 2>"${work_dir}/stderr.log" &
pid=$!

ready=0
for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "${pid}" >/dev/null 2>&1; then
    echo "BACKEND_NATIVE_EXITED=YES" >&2
    cat "${work_dir}/stderr.log" >&2 || true
    exit 1
  fi
  sleep 1
done
if [[ "${ready}" != 1 ]]; then
  echo "BACKEND_NATIVE_HEALTH_TIMEOUT=YES" >&2
  cat "${work_dir}/stderr.log" >&2 || true
  exit 1
fi
curl -fsS "http://127.0.0.1:${PORT}/readinessz" >/dev/null || {
  echo "BACKEND_NATIVE_READINESS=FAIL" >&2
  cat "${work_dir}/stderr.log" >&2 || true
  exit 1
}
echo "BACKEND_NATIVE_HEALTH=PASS"
echo "BACKEND_NATIVE_READINESS=PASS"
