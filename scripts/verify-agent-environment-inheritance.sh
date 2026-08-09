#!/usr/bin/env bash
# Verify a three-agent Trail lane chain with real Cargo and npm environments.
# The fixture and all Trail state are disposable and live on /Volumes/Workspace.
set -euo pipefail

trail_repo=${TRAIL_REPO:-/Users/haipingfu/Github/Trail}
trail_bin=${TRAIL_BIN:-/Volumes/Workspace/crabbuild-target/trail-env-inheritance/release/trail}
trail_target_dir=${TRAIL_TARGET_DIR:-/Volumes/Workspace/crabbuild-target/trail-env-inheritance}
external_root=${TRAIL_EXTERNAL_ROOT:-/Volumes/Workspace/Github/crabbuild}
tag=${TRAIL_ENV_E2E_TAG:-techain}
workdir_mode=${TRAIL_ENV_E2E_WORKDIR_MODE:-nfs-cow}
fixture_source="$trail_repo/scripts/fixtures/lane-environment"
workspace="$external_root/$tag"
logs="$external_root/${tag}-logs"

say() {
  printf '\n== %s ==\n' "$1"
}

run() {
  printf '\n$'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

run_capture() {
  local output=$1
  shift
  printf '\n$'
  printf ' %q' "$@"
  printf ' > %q\n' "$output"
  "$@" >"$output" 2>&1
  cat "$output"
}

run_capture_json() {
  local output=$1
  local error_output="${output%.json}.stderr"
  shift
  printf '\n$'
  printf ' %q' "$@"
  printf ' > %q 2> %q\n' "$output" "$error_output"
  "$@" >"$output" 2>"$error_output"
  cat "$error_output"
  cat "$output"
}

if [[ "$(uname -s)" != Darwin || "$workdir_mode" != nfs-cow ]]; then
  printf 'error: this qualification uses macOS nfs-cow; set up a macOS run with '
  printf 'TRAIL_ENV_E2E_WORKDIR_MODE=nfs-cow\n' >&2
  exit 2
fi

if [[ ! -d /Volumes/Workspace || ! -w /Volumes/Workspace ]]; then
  printf 'error: /Volumes/Workspace is unavailable or not writable; refusing local disk\n' >&2
  exit 2
fi
if [[ ! -d "$trail_repo/.git" ]]; then
  printf 'error: Trail checkout is missing: %s\n' "$trail_repo" >&2
  exit 2
fi
if [[ ! -d "$fixture_source" ]]; then
  printf 'error: fixture is missing: %s\n' "$fixture_source" >&2
  exit 2
fi

mkdir -p "$trail_target_dir" "$external_root"
if [[ ! -w "$trail_target_dir" ]]; then
  printf 'error: target directory is not writable: %s\n' "$trail_target_dir" >&2
  exit 2
fi
if [[ -e "$workspace" || -e "$logs" ]]; then
  printf 'error: refusing to overwrite existing disposable path(s):\n' >&2
  printf '  %s\n  %s\n' "$workspace" "$logs" >&2
  printf 'choose another TRAIL_ENV_E2E_TAG\n' >&2
  exit 2
fi

say "Build and identify the Trail binary"
if [[ "${TRAIL_BUILD:-1}" == 1 ]]; then
  (
    cd "$trail_repo"
    CARGO_TARGET_DIR="$trail_target_dir" cargo build -p trail --release --locked
  )
fi
[[ -x "$trail_bin" ]] || {
  printf 'error: Trail binary is not executable: %s\n' "$trail_bin" >&2
  exit 2
}
run "$trail_bin" --version

say "Create an isolated Cargo/npm qualification repository"
run cp -R "$fixture_source" "$workspace"
mkdir -p "$logs"
run git -C "$workspace" init -b main
run git -C "$workspace" config user.name "Trail environment experiment"
run git -C "$workspace" config user.email "trail-environment@example.invalid"
run git -C "$workspace" add .
run git -C "$workspace" commit -m "fixture: cargo and npm environment"

say "Initialize Trail and create Agent A"
run_capture "$logs/init.json" "$trail_bin" --workspace "$workspace" init --from-git --branch main --format json
run_capture "$logs/agent-a-spawn.json" "$trail_bin" --workspace "$workspace" lane spawn agent-a \
  --from main --workdir-mode "$workdir_mode" --format json

say "Agent A builds Cargo and installs npm once"
agent_a_command='set -eu
cargo build --offline --locked
test -x target/debug/trail-env-fixture
test "$(target/debug/trail-env-fixture)" = "trail environment fixture"
node -e '\''const _ = require("lodash"); if (_.camelCase("Agent A") !== "agentA") process.exit(7)'\''
test -f node_modules/lodash/lodash.js
printf "agent-a-private\\n" > target/agent-a-private.txt
printf "agent-a-private\\n" > node_modules/.agent-a-private'
run_capture "$logs/agent-a-exec.txt" "$trail_bin" --workspace "$workspace" lane exec agent-a -- \
  /bin/sh -lc "$agent_a_command"
grep -q 'Compiling trail-env-fixture' "$logs/agent-a-exec.txt"
grep -q 'added 1 package' "$logs/agent-a-exec.txt"
run_capture "$logs/agent-a-generation.json" "$trail_bin" --workspace "$workspace" env generation agent-a --format json

say "Agent B inherits Agent A's generation and starts"
run_capture "$logs/agent-b-spawn.json" "$trail_bin" --workspace "$workspace" lane spawn agent-b \
  --from agent-a --workdir-mode "$workdir_mode" --format json
# The first child attachment may report sync_all=succeeded while reusing the
# already-published layers. It must not run a compiler or package install.
run_capture_json "$logs/agent-b-sync.json" "$trail_bin" --workspace "$workspace" env sync all agent-b --format json
if grep -Eq 'Compiling trail-env-fixture|added 1 package' \
  "$logs/agent-b-sync.json" "$logs/agent-b-sync.stderr"; then
  printf 'error: Agent B rebuilt or reinstalled instead of reusing Agent A layers\n' >&2
  exit 1
fi
agent_b_command='set -eu
cargo build --offline --locked
test -x target/debug/trail-env-fixture
test "$(target/debug/trail-env-fixture)" = "trail environment fixture"
node -e '\''const _ = require("lodash"); if (_.camelCase("Agent B") !== "agentB") process.exit(7)'\''
test -f node_modules/lodash/lodash.js
test -f agent-a-source.txt
test ! -e target/agent-a-private.txt
test ! -e node_modules/.agent-a-private
printf "agent-b-private\\n" > target/agent-b-private.txt
printf "agent-b-private\\n" > node_modules/.agent-b-private'
run_capture_json "$logs/agent-b-exec.json" "$trail_bin" --workspace "$workspace" lane exec --format json agent-b -- \
  /bin/sh -lc "$agent_b_command"
if grep -Eq 'Compiling trail-env-fixture|added 1 package' \
  "$logs/agent-b-exec.json" "$logs/agent-b-exec.stderr"; then
  printf 'error: Agent B performed a full build or npm install\n' >&2
  exit 1
fi
run_capture "$logs/agent-b-generation.json" "$trail_bin" --workspace "$workspace" env generation agent-b --format json

say "Agent C inherits Agent B's generation and starts"
run_capture "$logs/agent-c-spawn.json" "$trail_bin" --workspace "$workspace" lane spawn agent-c \
  --from agent-b --workdir-mode "$workdir_mode" --format json
run_capture_json "$logs/agent-c-sync.json" "$trail_bin" --workspace "$workspace" env sync all agent-c --format json
if grep -Eq 'Compiling trail-env-fixture|added 1 package' \
  "$logs/agent-c-sync.json" "$logs/agent-c-sync.stderr"; then
  printf 'error: Agent C rebuilt or reinstalled instead of reusing Agent B layers\n' >&2
  exit 1
fi
agent_c_command='set -eu
cargo build --offline --locked
test -x target/debug/trail-env-fixture
test "$(target/debug/trail-env-fixture)" = "trail environment fixture"
node -e '\''const _ = require("lodash"); if (_.camelCase("Agent C") !== "agentC") process.exit(7)'\''
test -f node_modules/lodash/lodash.js
test -f agent-a-source.txt
test ! -e target/agent-a-private.txt
test ! -e node_modules/.agent-a-private
test ! -e target/agent-b-private.txt
test ! -e node_modules/.agent-b-private
printf "agent-c-private\\n" > target/agent-c-private.txt
printf "agent-c-private\\n" > node_modules/.agent-c-private'
run_capture_json "$logs/agent-c-exec.json" "$trail_bin" --workspace "$workspace" lane exec --format json agent-c -- \
  /bin/sh -lc "$agent_c_command"
if grep -Eq 'Compiling trail-env-fixture|added 1 package' \
  "$logs/agent-c-exec.json" "$logs/agent-c-exec.stderr"; then
  printf 'error: Agent C performed a full build or npm install\n' >&2
  exit 1
fi
run_capture "$logs/agent-c-generation.json" "$trail_bin" --workspace "$workspace" env generation agent-c --format json

say "Verify writable state remains lane-private"
run_capture "$logs/agent-a-final.json" "$trail_bin" --workspace "$workspace" lane exec --format json agent-a -- \
  /bin/sh -lc 'set -eu; test -f target/agent-a-private.txt; test -f node_modules/.agent-a-private; test ! -e target/agent-b-private.txt; test ! -e node_modules/.agent-b-private; test ! -e target/agent-c-private.txt; test ! -e node_modules/.agent-c-private'
run_capture "$logs/agent-b-final.json" "$trail_bin" --workspace "$workspace" lane exec --format json agent-b -- \
  /bin/sh -lc 'set -eu; test -f target/agent-b-private.txt; test -f node_modules/.agent-b-private; test ! -e target/agent-a-private.txt; test ! -e node_modules/.agent-a-private; test ! -e target/agent-c-private.txt; test ! -e node_modules/.agent-c-private'
run_capture "$logs/agent-c-final.json" "$trail_bin" --workspace "$workspace" lane exec --format json agent-c -- \
  /bin/sh -lc 'set -eu; test -f target/agent-c-private.txt; test -f node_modules/.agent-c-private; test ! -e target/agent-a-private.txt; test ! -e node_modules/.agent-a-private; test ! -e target/agent-b-private.txt; test ! -e node_modules/.agent-b-private'

say "Compare source roots, generations, and immutable layer identities"
python3 - "$logs" <<'PY'
import json
import pathlib
import sys

logs = pathlib.Path(sys.argv[1])


def read(name):
    with (logs / name).open() as handle:
        return json.load(handle)


generations = {
    name: read(f"agent-{name}-generation.json")
    for name in ("a", "b", "c")
}

def component(generation, component_id):
    for item in generation["components"]:
        if item["component_id"] == component_id:
            return item
    raise AssertionError(f"missing component {component_id}")


source_roots = {generation["source_root"] for generation in generations.values()}
assert len(source_roots) == 1, source_roots

cargo_layers = {
    component(generation, "cargo-target-seed")["layer_id"]
    for generation in generations.values()
}
node_layers = {component(generation, "node")["layer_id"] for generation in generations.values()}
assert len(cargo_layers) == 1, cargo_layers
assert len(node_layers) == 1, node_layers

# Spawning creates an inherited generation and env sync then activates a
# validated generation whose predecessor is that inherited record. The exact
# intermediate ID is intentionally not assumed; lineage must be present.
assert generations["b"]["predecessor_generation_id"]
assert generations["c"]["predecessor_generation_id"]

for name in ("b", "c"):
    receipt = read(f"agent-{name}-exec.json")
    phases = {phase["phase"]: phase["status"] for phase in receipt["lifecycle"]["phases"]}
    assert phases["sync_all"] == "skipped", (name, phases)
    assert receipt["exit_code"] == 0, receipt

for name in ("b", "c"):
    text = (logs / f"agent-{name}-exec.json").read_text() + (
        logs / f"agent-{name}-exec.stderr"
    ).read_text()
    assert "Compiling trail-env-fixture" not in text
    assert "added 1 package" not in text

print("source_root_shared=passed")
print(f"cargo_target_layer={next(iter(cargo_layers))}")
print(f"node_modules_layer={next(iter(node_layers))}")
print("agent_b_generation_predecessor=present")
print("agent_c_generation_predecessor=present")
print("agent_b_sync_all=skipped")
print("agent_c_sync_all=skipped")
print("agent_b_no_full_build_or_install=passed")
print("agent_c_no_full_build_or_install=passed")
print("lane_private_generated_state=passed")
PY

printf '\nALL AGENT ENVIRONMENT INHERITANCE CHECKS PASSED\n'
printf 'workspace=%s\nlogs=%s\n' "$workspace" "$logs"
