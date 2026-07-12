#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
cargo build -p trail
trail="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/trail"

write_spec() {
  local root="$1"
  shift
  local policy="${TRAIL_RECIPE_POLICY:-immutable_seed_private}"
  printf "declared input\n" >"${root}/input.txt"
  {
    printf "%s\n" "schema = \"trail.environment/v1\""
    printf "%s\n" "[environment]"
    printf "%s\n" "default_network = \"deny\""
    printf "%s\n" "default_scripts = \"deny\""
    printf "%s\n" "[[component]]"
    printf "%s\n" "id = \"generated.copy\""
    printf "%s\n" "adapter = \"trail/command@1\""
    printf "%s\n" "root = \".\""
    printf "%s\n" "kind = \"generated\""
    printf "%s\n" "[[component.input]]"
    printf "%s\n" "path = \"input.txt\""
    printf "%s\n" "role = \"identity\""
    printf "%s\n" "format = \"bytes\""
    printf "%s\n" "[component.build]"
    printf "command = ["
    local separator=""
    local argument
    for argument in "$@"; do
      argument="${argument//\\/\\\\}"
      argument="${argument//\"/\\\"}"
      printf "%s\"%s\"" "${separator}" "${argument}"
      separator=", "
    done
    printf "]\n"
    printf "%s\n" "cwd = \".\""
    printf "%s\n" "network = \"deny\""
    printf "%s\n" "scripts = \"deny\""
    printf "%s\n" "[[component.output]]"
    printf "%s\n" "source = \"generated\""
    printf "%s\n" "target = \".trail-generated/copy\""
    printf 'policy = "%s"\n' "${policy}"
    printf "%s\n" "portability = \"host\""
  } >"${root}/trail.environment.toml"
}

new_lane_workspace() {
  local root="$1"
  "${trail}" --workspace "${root}" init --working-tree >/dev/null
  "${trail}" --workspace "${root}" lane spawn recipe-a --from main --workdir-mode fuse-cow >/dev/null
}

success_root="$(mktemp -d)"
write_spec "${success_root}" cp input.txt generated/copied.txt
new_lane_workspace "${success_root}"
"${trail}" --workspace "${success_root}" lane spawn recipe-b --from main --workdir-mode fuse-cow >/dev/null
plan="$("${trail}" --workspace "${success_root}" --json env plan recipe-a --adapter command)"
grep -q "linux-landlock-seccomp" <<<"${plan}"
first="$("${trail}" --workspace "${success_root}" --json env sync recipe-a --adapter command)"
second="$("${trail}" --workspace "${success_root}" --json env sync recipe-b --adapter command)"
first_layer="$(sed -n "s/.*\"layer_id\": \"\([^\"]*\)\".*/\1/p" <<<"${first}" | head -1)"
second_layer="$(sed -n "s/.*\"layer_id\": \"\([^\"]*\)\".*/\1/p" <<<"${second}" | head -1)"
storage_path="$(sed -n "s/.*\"storage_path\": \"\([^\"]*\)\".*/\1/p" <<<"${first}" | head -1)"
test -n "${first_layer}"
test "${first_layer}" = "${second_layer}"
cmp "${success_root}/input.txt" "${storage_path}/copied.txt"

private_root="$(mktemp -d)"
TRAIL_RECIPE_POLICY=writable_private write_spec "${private_root}" cp input.txt generated/copied.txt
new_lane_workspace "${private_root}"
private_sync="$("${trail}" --workspace "${private_root}" --json env sync recipe-a --adapter command)"
python3 -c '
import json, sys
report = json.load(sys.stdin)
assert report["layers"] == [], report
output = report["generation"]["components"][0]["outputs"][0]
assert output["policy"] == "writable_private", output
assert output["layer_id"] is None, output
' <<<"${private_sync}"
"${trail}" --workspace "${private_root}" lane exec recipe-a -- \
  sh -c 'printf private-mutation >.trail-generated/copy/copied.txt'
"${trail}" --workspace "${private_root}" env sync recipe-a --adapter command >/dev/null
"${trail}" --workspace "${private_root}" lane exec recipe-a -- \
  sh -c 'grep -q private-mutation .trail-generated/copy/copied.txt'

multi_root="$(mktemp -d)"
write_spec "${multi_root}" touch generated/a.txt generated-b/b.txt
{
  printf "%s\n" "[[component.output]]"
  printf "%s\n" "name = \"beta\""
  printf "%s\n" "source = \"generated-b\""
  printf "%s\n" "target = \".trail-generated/beta\""
  printf "%s\n" "policy = \"immutable_seed_private\""
  printf "%s\n" "portability = \"host\""
} >>"${multi_root}/trail.environment.toml"
new_lane_workspace "${multi_root}"
"${trail}" --workspace "${multi_root}" lane spawn recipe-b --from main --workdir-mode fuse-cow >/dev/null
multi_plan="$("${trail}" --workspace "${multi_root}" --json env plan recipe-a --adapter command)"
test "$(grep -c '"output_path"' <<<"${multi_plan}")" -ge 3
multi_first="$("${trail}" --workspace "${multi_root}" --json env sync-all recipe-a)"
multi_second="$("${trail}" --workspace "${multi_root}" --json env sync recipe-b --adapter command)"
multi_first_layer="$(sed -n "s/.*\"layer_id\": \"\([^\"]*\)\".*/\1/p" <<<"${multi_first}" | head -1)"
multi_second_layer="$(sed -n "s/.*\"layer_id\": \"\([^\"]*\)\".*/\1/p" <<<"${multi_second}" | head -1)"
multi_storage="$(sed -n "s/.*\"storage_path\": \"\([^\"]*\)\".*/\1/p" <<<"${multi_first}" | head -1)"
test -f "${multi_storage}/outputs/0000/a.txt"
test -f "${multi_storage}/outputs/0001/b.txt"
test "${multi_first_layer}" = "${multi_second_layer}"
"${trail}" --workspace "${multi_root}" lane exec recipe-a -- \
  sh -c 'test -f .trail-generated/copy/a.txt && test -f .trail-generated/beta/b.txt && printf lane-a >.trail-generated/copy/a.txt'
"${trail}" --workspace "${multi_root}" lane exec recipe-b -- \
  sh -c 'test -f .trail-generated/copy/a.txt && test ! -s .trail-generated/copy/a.txt && test -f .trail-generated/beta/b.txt'

host_read_root="$(mktemp -d)"
write_spec "${host_read_root}" cp /etc/passwd generated/copied.txt
new_lane_workspace "${host_read_root}"
if "${trail}" --workspace "${host_read_root}" env sync recipe-a --adapter command >/dev/null 2>&1; then
  echo "restricted recipe unexpectedly read /etc/passwd" >&2
  exit 1
fi

write_root="$(mktemp -d)"
write_spec "${write_root}" cp input.txt escape.txt
new_lane_workspace "${write_root}"
if "${trail}" --workspace "${write_root}" env sync recipe-a --adapter command >/dev/null 2>&1; then
  echo "restricted recipe unexpectedly wrote outside its declared output" >&2
  exit 1
fi

network_root="$(mktemp -d)"
write_spec "${network_root}" curl --fail --max-time 1 http://127.0.0.1:9 -o generated/network.txt
new_lane_workspace "${network_root}"
if "${trail}" --workspace "${network_root}" env sync recipe-a --adapter command >/dev/null 2>&1; then
  echo "restricted recipe unexpectedly used a network socket" >&2
  exit 1
fi

shell_root="$(mktemp -d)"
write_spec "${shell_root}" sh -c true
new_lane_workspace "${shell_root}"
if "${trail}" --workspace "${shell_root}" env plan recipe-a --adapter command >/dev/null 2>&1; then
  echo "restricted recipe unexpectedly accepted a shell" >&2
  exit 1
fi

printf "linux-command-recipe shared-layer=%s multi-output-layer=%s writable-private=verified host-read=denied undeclared-write=denied network=denied shell=denied\n" "${first_layer}" "${multi_first_layer}"
