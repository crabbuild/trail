#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

cargo build -p trail
cargo build -p trail-environment-adapter-sdk --example generated-copy-adapter --example mounted-initializer-adapter --example mounted-fixture-tool --example cache-adapter --example cache-fixture-tool --example adversarial-adapter --example fixture-sign-adapter
trail="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/trail"
example_dir="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/examples"

root="$(mktemp -d)"
packages="$(mktemp -d)"
tool_bin="$(mktemp -d)"
cleanup() {
  chmod -R u+w "${root}" "${packages}" "${tool_bin}" 2>/dev/null || true
  rm -rf "${root}" "${packages}" "${tool_bin}"
}
trap cleanup EXIT
cp "${example_dir}/mounted-fixture-tool" "${tool_bin}/mounted-fixture-tool"
chmod +x "${tool_bin}/mounted-fixture-tool"
cp "${example_dir}/cache-fixture-tool" "${tool_bin}/cache-fixture-tool"
chmod +x "${tool_bin}/cache-fixture-tool"
export PATH="${tool_bin}:${PATH}"
printf "plugin marker\n" >"${root}/copy.adapter"
printf "success\n" >"${root}/mounted.adapter"
printf "lane-a\n" >"${root}/cache.adapter"
printf "declared input\n" >"${root}/input.txt"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

write_package() {
  local directory="$1"
  local identity="$2"
  local selector="$3"
  local executable_source="$4"
  local timeout_ms="$5"
  local response_bytes="$6"
  mkdir -p "${directory}"
  cp "${executable_source}" "${directory}/adapter-plugin"
  chmod +x "${directory}/adapter-plugin"
  local digest
  digest="$(sha256_file "${directory}/adapter-plugin")"
  {
    printf '%s\n' 'schema = "trail.environment-adapter-package/v1"'
    printf '%s\n' '[adapter]'
    printf 'canonical_identity = "%s"\n' "${identity}"
    printf '%s\n' 'implementation_version = "1.0.0"'
    printf 'selectors = ["%s", "%s"]\n' "${identity}" "${selector}"
    printf '%s\n' 'kind = "generated"'
    printf 'layer_adapter_name = "%s"\n' "${selector}"
    printf '%s\n' 'discovery_markers = ["copy.adapter"]'
    printf '%s\n' 'stability = "experimental"'
    printf 'description = "Fixture for %s"\n' "${identity}"
    printf '%s\n' '[executable]'
    printf '%s\n' 'path = "adapter-plugin"'
    printf 'sha256 = "%s"\n' "${digest}"
    printf '%s\n' '[permissions]'
    printf '%s\n' 'read_patterns = ["copy.adapter", "input.txt"]'
    printf '%s\n' 'max_input_files = 8'
    printf '%s\n' 'max_input_bytes = 1048576'
    printf 'timeout_ms = %s\n' "${timeout_ms}"
    printf 'max_response_bytes = %s\n' "${response_bytes}"
  } >"${directory}/trail-adapter.toml"
}

write_mounted_package() {
  local directory="$1"
  mkdir -p "${directory}"
  cp "${example_dir}/mounted-initializer-adapter" "${directory}/adapter-plugin"
  chmod +x "${directory}/adapter-plugin"
  local digest
  digest="$(sha256_file "${directory}/adapter-plugin")"
  {
    printf '%s\n' 'schema = "trail.environment-adapter-package/v1"'
    printf '%s\n' '[adapter]'
    printf '%s\n' 'canonical_identity = "example/mounted@1"'
    printf '%s\n' 'implementation_version = "1.0.0"'
    printf '%s\n' 'selectors = ["example/mounted@1", "example-mounted"]'
    printf '%s\n' 'kind = "generated"'
    printf '%s\n' 'layer_adapter_name = "example-mounted"'
    printf '%s\n' 'discovery_markers = ["mounted.adapter"]'
    printf '%s\n' 'protocols = ["trail.environment-adapter/v2"]'
    printf '%s\n' 'stability = "experimental"'
    printf '%s\n' 'description = "Mounted initializer protocol-v2 fixture"'
    printf '%s\n' '[executable]'
    printf '%s\n' 'path = "adapter-plugin"'
    printf 'sha256 = "%s"\n' "${digest}"
    printf '%s\n' '[permissions]'
    printf '%s\n' 'read_patterns = ["mounted.adapter"]'
    printf '%s\n' 'max_input_files = 8'
    printf '%s\n' 'max_input_bytes = 1048576'
    printf '%s\n' 'timeout_ms = 5000'
    printf '%s\n' 'max_response_bytes = 1048576'
  } >"${directory}/trail-adapter.toml"
}

write_cache_package() {
  local directory="$1"
  mkdir -p "${directory}"
  cp "${example_dir}/cache-adapter" "${directory}/adapter-plugin"
  chmod +x "${directory}/adapter-plugin"
  local digest
  digest="$(sha256_file "${directory}/adapter-plugin")"
  {
    printf '%s\n' 'schema = "trail.environment-adapter-package/v1"'
    printf '%s\n' '[adapter]'
    printf '%s\n' 'canonical_identity = "example/cache@1"'
    printf '%s\n' 'implementation_version = "1.0.0"'
    printf '%s\n' 'selectors = ["example/cache@1", "example-cache"]'
    printf '%s\n' 'kind = "generated"'
    printf '%s\n' 'layer_adapter_name = "example-cache"'
    printf '%s\n' 'discovery_markers = ["cache.adapter"]'
    printf '%s\n' 'protocols = ["trail.environment-adapter/v2"]'
    printf '%s\n' 'stability = "experimental"'
    printf '%s\n' 'description = "Host-owned cache protocol-v2 fixture"'
    printf '%s\n' '[executable]'
    printf '%s\n' 'path = "adapter-plugin"'
    printf 'sha256 = "%s"\n' "${digest}"
    printf '%s\n' '[permissions]'
    printf '%s\n' 'read_patterns = ["cache.adapter"]'
    printf '%s\n' 'max_input_files = 8'
    printf '%s\n' 'max_input_bytes = 1048576'
    printf '%s\n' 'timeout_ms = 5000'
    printf '%s\n' 'max_response_bytes = 1048576'
  } >"${directory}/trail-adapter.toml"
}

json_field() {
  local field="$1"
  python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "${field}"
}

first_line() {
  python3 -c 'import sys; lines=sys.stdin.read().splitlines(); print(lines[0] if lines else "")'
}

sync_layer_field() {
  local field="$1"
  python3 -c 'import json,sys; print(json.load(sys.stdin)["layers"][0][sys.argv[1]])' "${field}"
}

assert_fails() {
  local message="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    printf '%s\n' "${message}" >&2
    exit 1
  fi
}

write_package "${packages}/copy" "example/copy@1" "example-copy" \
  "${example_dir}/generated-copy-adapter" 5000 1048576
write_mounted_package "${packages}/mounted"
write_cache_package "${packages}/cache"

"${trail}" --workspace "${root}" init --working-tree >/dev/null
inspection="$("${trail}" --workspace "${root}" --json env plugin inspect "${packages}/copy")"
payload_digest="$(json_field payload_digest <<<"${inspection}")"
"${example_dir}/fixture-sign-adapter" \
  "example-publisher" \
  "0707070707070707070707070707070707070707070707070707070707070707" \
  "${payload_digest}" \
  "${packages}/copy/trail-adapter.sig" \
  "${packages}/copy/publisher-key.toml"
assert_fails "signed adapter unexpectedly installed before publisher trust" \
  "${trail}" --workspace "${root}" env plugin install "${packages}/copy"
trust_json="$("${trail}" --workspace "${root}" --json env plugin trust add "${packages}/copy/publisher-key.toml")"
publisher_key_id="$(json_field key_id <<<"${trust_json}")"
mode="fuse-cow"
if [[ "$(uname -s)" == "Darwin" ]]; then
  mode="nfs-cow"
fi
"${trail}" --workspace "${root}" lane spawn plugin-a --from main --workdir-mode "${mode}" >/dev/null
"${trail}" --workspace "${root}" lane spawn plugin-b --from main --workdir-mode "${mode}" >/dev/null
"${trail}" --workspace "${root}" lane spawn plugin-private --from main --workdir-mode "${mode}" >/dev/null
"${trail}" --workspace "${root}" lane spawn plugin-mounted-a --from main --workdir-mode "${mode}" >/dev/null
"${trail}" --workspace "${root}" lane spawn plugin-mounted-b --from main --workdir-mode "${mode}" >/dev/null
"${trail}" --workspace "${root}" lane spawn plugin-mounted-kill --from main --workdir-mode "${mode}" >/dev/null

install_json="$("${trail}" --workspace "${root}" --json env plugin install "${packages}/copy")"
distribution="$(json_field distribution_digest <<<"${install_json}")"
grep -q '"trust": "publisher_signed"' <<<"${install_json}"
grep -q '"certification_tier": "publisher-authenticated-experimental"' <<<"${install_json}"
catalog="$("${trail}" --workspace "${root}" --json env adapters)"
grep -q '"canonical_identity": "example/copy@1"' <<<"${catalog}"
grep -q '"source": "plugin"' <<<"${catalog}"
discovery="$("${trail}" --workspace "${root}" --json env discover plugin-a)"
grep -q '"component_id": "plugin.copy"' <<<"${discovery}"
plan="$("${trail}" --workspace "${root}" --json env plan plugin-a --adapter example/copy@1)"
grep -q '"sandbox": "' <<<"${plan}"
grep -q '"network": "deny"' <<<"${plan}"
first="$("${trail}" --workspace "${root}" --json env sync plugin-a --adapter example/copy@1)"
second="$("${trail}" --workspace "${root}" --json env sync plugin-b --adapter example/copy@1)"
first_layer="$(sync_layer_field layer_id <<<"${first}")"
second_layer="$(sync_layer_field layer_id <<<"${second}")"
storage="$(sync_layer_field storage_path <<<"${first}")"
test "${first_layer}" = "${second_layer}"
cmp "${root}/input.txt" "${storage}/copied.txt"
"${trail}" --workspace "${root}" lane exec plugin-private -- \
  sh -c 'printf writable_private >copy.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-private -m "select private plugin output" >/dev/null
private_sync="$("${trail}" --workspace "${root}" --json env sync plugin-private --adapter example/copy@1)"
python3 -c '
import json, sys
report = json.load(sys.stdin)
assert report["layers"] == [], report
output = report["generation"]["components"][0]["outputs"][0]
assert output["policy"] == "writable_private", output
assert output["layer_id"] is None, output
' <<<"${private_sync}"
"${trail}" --workspace "${root}" lane exec plugin-private -- \
  sh -c 'printf private-plugin-mutation >.trail-generated/plugin-copy/copied.txt'
"${trail}" --workspace "${root}" env sync plugin-private --adapter example/copy@1 >/dev/null
"${trail}" --workspace "${root}" lane exec plugin-private -- \
  sh -c 'grep -q private-plugin-mutation .trail-generated/plugin-copy/copied.txt'
"${trail}" --workspace "${root}" lane exec plugin-a -- \
  sh -c 'printf lane-a >.trail-generated/plugin-copy/copied.txt'
"${trail}" --workspace "${root}" lane exec plugin-b -- \
  sh -c 'grep -q "declared input" .trail-generated/plugin-copy/copied.txt'
"${trail}" --workspace "${root}" lane exec plugin-a -- \
  sh -c 'printf changed-input >input.txt'
"${trail}" --workspace "${root}" lane checkpoint plugin-a -m "change plugin input" >/dev/null
readiness="$("${trail}" --workspace "${root}" --json lane readiness plugin-a)"
grep -q 'dependency_environment_stale' <<<"${readiness}"
status="$("${trail}" --workspace "${root}" --json env status plugin-a)"
grep -q '"status": "stale"' <<<"${status}"

"${trail}" --workspace "${root}" env plugin install "${packages}/cache" >/dev/null
"${trail}" --workspace "${root}" lane exec plugin-b -- sh -c 'printf lane-b >cache.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-b -m "give cache fixture a distinct component key" >/dev/null
cache_plan="$("${trail}" --workspace "${root}" --json env plan plugin-a --adapter example/cache@1)"
grep -q '"name": "fixture-store"' <<<"${cache_plan}"
grep -q '"protocol": "content_store"' <<<"${cache_plan}"
grep -q '"access": "host_exclusive"' <<<"${cache_plan}"
cache_a="$("${trail}" --workspace "${root}" --json env sync plugin-a --adapter example/cache@1)"
cache_b="$("${trail}" --workspace "${root}" --json env sync plugin-b --adapter example/cache@1)"
cache_a_namespace="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["generation"]["components"][0]["caches"][0]["namespace_id"])' <<<"${cache_a}")"
cache_b_namespace="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["generation"]["components"][0]["caches"][0]["namespace_id"])' <<<"${cache_b}")"
test "${cache_a_namespace}" = "${cache_b_namespace}"
cache_a_observation="$("${trail}" --workspace "${root}" lane exec plugin-a -- sh -c 'cat .trail-generated/plugin-cache/cache-observation.txt' | first_line | tr -d '\r')"
cache_b_observation="$("${trail}" --workspace "${root}" lane exec plugin-b -- sh -c 'cat .trail-generated/plugin-cache/cache-observation.txt' | first_line | tr -d '\r')"
test "${cache_a_observation}" = "${cache_a_namespace}|1"
test "${cache_b_observation}" = "${cache_b_namespace}|2"
test "$(tr -d '\r\n' <"${root}/.trail/cache/namespaces/${cache_a_namespace}/counter")" = "2"
"${trail}" --workspace "${root}" lane exec plugin-a -- sh -c 'printf escape >cache.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-a -m "attempt plugin cache namespace escape" >/dev/null
assert_fails "plugin cache write unexpectedly escaped its namespace" \
  "${trail}" --workspace "${root}" env sync plugin-a --adapter example/cache@1
test ! -e "${root}/.trail/cache/namespaces/plugin-cache-escape"
"${trail}" --workspace "${root}" lane exec plugin-a -- \
  sh -c 'grep -q "|1$" .trail-generated/plugin-cache/cache-observation.txt'

"${trail}" --workspace "${root}" env plugin install "${packages}/mounted" >/dev/null
mounted_catalog="$("${trail}" --workspace "${root}" --json env adapters)"
grep -q 'trail.environment-adapter/v2' <<<"${mounted_catalog}"
mounted_plan="$("${trail}" --workspace "${root}" --json env plan plugin-mounted-a --adapter example/mounted@1)"
grep -q '"phase": "mounted_initialization"' <<<"${mounted_plan}"
if grep -q '"phase": "staging"' <<<"${mounted_plan}"; then
  printf '%s\n' "mounted-only plugin unexpectedly gained a staging action" >&2
  exit 1
fi
mounted_a="$("${trail}" --workspace "${root}" --json env sync plugin-mounted-a --adapter example/mounted@1)"
mounted_b="$("${trail}" --workspace "${root}" --json env sync plugin-mounted-b --adapter example/mounted@1)"
python3 -c 'import json,sys; assert json.load(sys.stdin)["layers"] == []' <<<"${mounted_a}"
python3 -c 'import json,sys; assert json.load(sys.stdin)["layers"] == []' <<<"${mounted_b}"
mounted_a_pwd="$("${trail}" --workspace "${root}" lane exec plugin-mounted-a -- pwd | first_line | tr -d '\r')"
mounted_b_pwd="$("${trail}" --workspace "${root}" lane exec plugin-mounted-b -- pwd | first_line | tr -d '\r')"
mounted_a_recorded="$("${trail}" --workspace "${root}" lane exec plugin-mounted-a -- sh -c 'cat .trail-generated/plugin-mounted/initialized.txt' | first_line | tr -d '\r')"
mounted_b_recorded="$("${trail}" --workspace "${root}" lane exec plugin-mounted-b -- sh -c 'cat .trail-generated/plugin-mounted/initialized.txt' | first_line | tr -d '\r')"
mounted_a_recorded="${mounted_a_recorded%|success}"
mounted_b_recorded="${mounted_b_recorded%|success}"
test "${mounted_a_recorded}" = "${mounted_a_pwd}"
test "${mounted_b_recorded}" = "${mounted_b_pwd}"
test "${mounted_a_recorded}" != "${mounted_b_recorded}"
"${trail}" --workspace "${root}" lane exec plugin-mounted-a -- \
  sh -c 'printf lane-a-private >.trail-generated/plugin-mounted/initialized.txt'
"${trail}" --workspace "${root}" env sync plugin-mounted-a --adapter example/mounted@1 >/dev/null
"${trail}" --workspace "${root}" lane exec plugin-mounted-a -- \
  sh -c 'grep -q lane-a-private .trail-generated/plugin-mounted/initialized.txt'
"${trail}" --workspace "${root}" lane exec plugin-mounted-b -- \
  sh -c 'test "$(cat .trail-generated/plugin-mounted/initialized.txt)" != lane-a-private'
"${trail}" --workspace "${root}" lane exec plugin-mounted-a -- sh -c 'printf fail >mounted.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-mounted-a -m "fail mounted plugin action" >/dev/null
assert_fails "failed mounted plugin action unexpectedly activated" \
  "${trail}" --workspace "${root}" env sync plugin-mounted-a --adapter example/mounted@1
"${trail}" --workspace "${root}" lane exec plugin-mounted-a -- \
  sh -c 'grep -q lane-a-private .trail-generated/plugin-mounted/initialized.txt && test ! -e .trail-generated/plugin-mounted/partial.txt'
"${trail}" --workspace "${root}" lane exec plugin-mounted-b -- sh -c 'printf source_write >mounted.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-mounted-b -m "attempt mounted source write" >/dev/null
assert_fails "mounted plugin source write unexpectedly escaped its declared output" \
  "${trail}" --workspace "${root}" env sync plugin-mounted-b --adapter example/mounted@1
"${trail}" --workspace "${root}" lane exec plugin-mounted-b -- \
  sh -c 'test ! -e source-leak.txt && test -f .trail-generated/plugin-mounted/initialized.txt'
"${trail}" --workspace "${root}" lane exec plugin-mounted-b -- sh -c 'printf source_read >mounted.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-mounted-b -m "attempt undeclared mounted source read" >/dev/null
assert_fails "mounted plugin undeclared source read unexpectedly succeeded" \
  "${trail}" --workspace "${root}" env sync plugin-mounted-b --adapter example/mounted@1
"${trail}" --workspace "${root}" lane exec plugin-mounted-b -- \
  sh -c 'test ! -e .trail-generated/plugin-mounted/leaked.txt && test -f .trail-generated/plugin-mounted/initialized.txt'
"${trail}" --workspace "${root}" env sync plugin-mounted-kill --adapter example/mounted@1 >/dev/null
"${trail}" --workspace "${root}" lane exec plugin-mounted-kill -- \
  sh -c 'printf kill-predecessor >.trail-generated/plugin-mounted/initialized.txt; printf hang >mounted.adapter'
"${trail}" --workspace "${root}" lane checkpoint plugin-mounted-kill -m "kill active mounted plugin action" >/dev/null
"${trail}" --workspace "${root}" env sync plugin-mounted-kill --adapter example/mounted@1 \
  >"${packages}/mounted-kill.stdout" 2>"${packages}/mounted-kill.stderr" &
mounted_sync_pid=$!
mounted_ready=""
for _ in $(seq 1 200); do
  mounted_ready="$(find "${root}/.trail/cache/staging" -path '*/process/*/home/running' -type f -print -quit 2>/dev/null || true)"
  if [[ -n "${mounted_ready}" ]]; then
    break
  fi
  sleep 0.05
done
if [[ -z "${mounted_ready}" ]]; then
  kill -9 "${mounted_sync_pid}" 2>/dev/null || true
  wait "${mounted_sync_pid}" 2>/dev/null || true
  printf '%s\n' "mounted plugin did not reach its active-command kill point" >&2
  exit 1
fi
mounted_child_pid="$(tr -d '\r\n' <"${mounted_ready}")"
kill -9 "${mounted_sync_pid}"
wait "${mounted_sync_pid}" 2>/dev/null || true
for _ in $(seq 1 200); do
  if ! kill -0 "${mounted_child_pid}" 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if kill -0 "${mounted_child_pid}" 2>/dev/null; then
  printf '%s\n' "mounted plugin action survived Trail process death" >&2
  kill -9 "${mounted_child_pid}" 2>/dev/null || true
  exit 1
fi
"${trail}" --workspace "${root}" env status plugin-mounted-kill >/dev/null
"${trail}" --workspace "${root}" lane exec plugin-mounted-kill -- \
  sh -c 'grep -q kill-predecessor .trail-generated/plugin-mounted/initialized.txt && test ! -e .trail-generated/plugin-mounted/partial.txt'
if find "${root}/.trail/cache/staging" -maxdepth 1 -name 'mounted-environment-*' -print -quit | grep -q .; then
  printf '%s\n' "recovery left an abandoned mounted plugin candidate" >&2
  exit 1
fi

for behavior in hang crash oversized malformed child memory; do
  response_bytes=1048576
  timeout_ms=1000
  if [[ "${behavior}" == "hang" ]]; then
    timeout_ms=100
  fi
  write_package "${packages}/${behavior}" "example/${behavior}@1" "example-${behavior}" \
    "${example_dir}/adversarial-adapter" "${timeout_ms}" "${response_bytes}"
  "${trail}" --workspace "${root}" env plugin install "${packages}/${behavior}" >/dev/null
  assert_fails "adversarial ${behavior} adapter unexpectedly succeeded" \
    "${trail}" --workspace "${root}" env plan plugin-a --adapter "example/${behavior}@1"
done

removed="$("${trail}" --workspace "${root}" --json env plugin remove example/copy@1)"
grep -q "${distribution}" <<<"${removed}"
if "${trail}" --workspace "${root}" --json env adapters | grep -q 'example/copy@1'; then
  printf '%s\n' "removed adapter remained active" >&2
  exit 1
fi

reinstalled="$("${trail}" --workspace "${root}" --json env plugin install "${packages}/copy")"
package_path="$(json_field package_path <<<"${reinstalled}")"
printf 'tamper\n' >>"${package_path}/adapter-plugin"
assert_fails "tampered adapter executable remained trusted" \
  "${trail}" --workspace "${root}" env adapters
"${trail}" --workspace "${root}" env plugin install "${packages}/copy" >/dev/null
"${trail}" --workspace "${root}" env plugin trust remove "${publisher_key_id}" >/dev/null
assert_fails "revoked publisher key left signed adapter active" \
  "${trail}" --workspace "${root}" env adapters

printf 'environment-adapter-plugin distribution=%s shared-layer=%s external-cache=%s:shared-host-exclusive-and-sandbox-contained writable-private=verified mounted-v2=isolated-and-atomic active-command-kill=terminated-and-recovered declared-read=allowed undeclared-read-write=denied private-copy-up=isolated stale-refresh=verified publisher-signature=verified revocation=fail-closed timeout=denied memory=denied crash=denied oversized=denied malformed=denied child-process=denied tamper=denied\n' \
  "${distribution}" "${first_layer}" "${cache_a_namespace}"
