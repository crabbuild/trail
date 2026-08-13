#!/usr/bin/env bash
# Qualify one pinned real repository through an Agent A -> B -> C native-COW
# handoff. This is an opt-in release evidence gate, not a networked unit test.
set -euo pipefail
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
SEMANTIC_EDITOR=$SCRIPT_DIR/edit-real-framework-semantic.py

die() {
  echo "verify-real-framework-handoff: $*" >&2
  exit 64
}

[[ $# == 1 ]] || die "usage: $0 <go|go-workspace|yarn|bun|pnpm|npm|python|uv|cmake|cmake-modern>"
framework=$1
: "${TRAIL_BIN:?set TRAIL_BIN to the candidate Trail executable}"
: "${TRAIL_FRAMEWORK_EVIDENCE_DIR:?set TRAIL_FRAMEWORK_EVIDENCE_DIR to a new output directory}"
[[ $TRAIL_BIN == /* ]] || die "TRAIL_BIN must be absolute"
[[ -x $TRAIL_BIN ]] || die "TRAIL_BIN is not executable: $TRAIL_BIN"
[[ -f $SEMANTIC_EDITOR ]] || die "semantic editor is missing: $SEMANTIC_EDITOR"
[[ $TRAIL_FRAMEWORK_EVIDENCE_DIR == /* ]] || die "TRAIL_FRAMEWORK_EVIDENCE_DIR must be absolute"
[[ ! -e $TRAIL_FRAMEWORK_EVIDENCE_DIR ]] || die "evidence directory already exists"
qualification_root=${TRAIL_FRAMEWORK_WORK_ROOT:-$TRAIL_FRAMEWORK_EVIDENCE_DIR.work}
[[ $qualification_root == /* ]] || die "TRAIL_FRAMEWORK_WORK_ROOT must be absolute when set"
[[ ! -e $qualification_root ]] || die "qualification work directory already exists"

# NFS remains the installation-free macOS qualification backend. The explicit
# override keeps FUSE and Dokan independently qualifiable on prepared hosts.
workdir_mode=${TRAIL_FRAMEWORK_WORKDIR_MODE:-nfs-cow}
case "$workdir_mode" in
  fuse-cow|nfs-cow|dokan-cow) ;;
  *) die "TRAIL_FRAMEWORK_WORKDIR_MODE must be fuse-cow, nfs-cow, or dokan-cow" ;;
esac

case "$framework" in
  go)
    repository=https://github.com/etcd-io/bbolt.git
    revision=55cb34b031c9855defb6c52db560a610f85bf5c3
    component_selector=go-vendor
    component_id=go-vendor
    ;;
  go-workspace)
    repository=https://github.com/oxia-db/oxia.git
    revision=8494f2a8bc4a36d5a93cd1c4101639be7a040163
    component_selector=go-vendor-workspace
    component_id=go-vendor
    ;;
  yarn)
    repository=https://github.com/jonschlinkert/is-odd.git
    revision=b8fc75839e341f23e2d7cb2d4b6a173ccbc1e364
    component_selector=node
    component_id=node
    ;;
  bun)
    repository=https://github.com/nozomio-labs/nia-cli.git
    revision=3ebf0b0bb62ff6a73d630232b2b03c8bde30fe86
    component_selector=node
    component_id=node
    ;;
  pnpm)
    repository=https://github.com/Polymarket/clob-client-v2.git
    revision=f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69
    component_selector=node
    component_id=node
    ;;
  npm)
    repository=https://github.com/uuidjs/uuid.git
    revision=b1da338815af4d919295eacb33aae340e372232a
    component_selector=node
    component_id=node
    ;;
  python)
    repository=https://github.com/python-tap/tappy.git
    revision=d050f1c52fcc51a145652aa57ed54856070abfdc
    component_selector=python
    component_id=python-venv
    ;;
  uv)
    repository=https://github.com/pyprojectx/pyprojectx.git
    revision=e615df93474fdd7b1c5d798c8d521499b3f87c42
    component_selector=python
    component_id=python-venv
    ;;
  cmake)
    repository=https://github.com/google/leveldb.git
    revision=7ee830d02b623e8ffe0b95d59a74db1e58da04c5
    component_selector=cmake-build
    component_id=cmake-build
    ;;
  cmake-modern)
    repository=https://github.com/CLIUtils/CLI11.git
    revision=60492bddb50422f32cfa33c1365b96ebee4205ca
    component_selector=cmake-build
    component_id=cmake-build
    : "${TRAIL_CMAKE_CONFIGURE_PRESET:=dev}"
    export TRAIL_CMAKE_CONFIGURE_PRESET
    command -v ninja >/dev/null || die "cmake-modern qualification requires Ninja"
    command -v ccache >/dev/null || die "cmake-modern qualification requires ccache"
    ;;
  *) die "unsupported framework: $framework" ;;
esac

mkdir -p "$TRAIL_FRAMEWORK_EVIDENCE_DIR/raw"
repository_root=$qualification_root/repository
mkdir -p "$qualification_root"

git -C "$qualification_root" init -q repository
git -C "$repository_root" remote add origin "$repository"
git -C "$repository_root" fetch -q --depth=1 origin "$revision"
git -C "$repository_root" checkout -q --detach FETCH_HEAD
[[ $(git -C "$repository_root" rev-parse HEAD) == "$revision" ]] || die "pinned revision mismatch"
python3_bin=$(command -v python3) || die "python3 is required"
run_json() {
  local output=$1
  shift
  local destination=$TRAIL_FRAMEWORK_EVIDENCE_DIR/raw/$output.json
  local pending=$destination.pending
  if ! "$TRAIL_BIN" --format json "$@" >"$pending"; then
    rm -f -- "$pending"
    return 1
  fi
  mv -- "$pending" "$destination"
}

run_edit() {
  local lane=$1
  run_json "edit-$lane" lane exec "$lane" -- \
    "$python3_bin" "$SEMANTIC_EDITOR" edit "$framework" "$lane"
}

run_framework_precheck() {
  local lane=$1
  local expected=$2
  local go_package
  case "$framework" in
    go|go-workspace)
      if [[ $framework == go ]]; then
        go_package=./version
      else
        go_package=./common
      fi
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify "$3" "$4"
         exec "$TRAIL_GO" test "$5" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$framework" "$expected" \
        "$go_package"
      ;;
    yarn)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify yarn "$3"
         if test "$3" = baseline; then
           exec "$TRAIL_YARN" mocha test.js --grep "should return true if the number is odd" 1>&2
         fi
         exec "$TRAIL_YARN" mocha test.js --grep "Trail qualification marker" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    bun)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify bun "$3"
         if test "$3" = baseline; then
           exec "$TRAIL_BUN" test tests/setup.test.ts 1>&2
         fi
         exec "$TRAIL_BUN" test tests/app.test.ts -t "Trail qualification marker" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    pnpm)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify pnpm "$3"
         "$TRAIL_PNPM" exec tsc --noEmit 1>&2
         if test "$3" = baseline; then
           exec "$TRAIL_PNPM" exec vitest run tests/http-helpers/index.test.ts 1>&2
         fi
         exec "$TRAIL_PNPM" exec vitest run tests/http-helpers/index.test.ts -t "Trail qualification marker" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    npm)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify npm "$3"
         "$TRAIL_NPM" run build -- --no-pack 1>&2
         exec "$TRAIL_NODE" --test --enable-source-maps dist-node/test/version.test.js 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    python)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify python "$3"
         "$TRAIL_VENV_PYTHON" -c '\''import os,sys; prefix=os.path.realpath(os.environ["VIRTUAL_ENV"]); assert os.path.realpath(sys.prefix) == prefix; assert os.path.commonpath([prefix, os.path.realpath(os.environ["TRAIL_VENV_PYTHON"])]) == prefix'\''
         if test "$3" = baseline; then
           exec "$TRAIL_VENV_PYTHON" -m pytest -q tests/test_line.py 1>&2
         fi
         exec "$TRAIL_VENV_PYTHON" -m pytest -q tests/test_line.py -k trail_qualification_marker 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    uv)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify uv "$3"
         "$TRAIL_VENV_PYTHON" -c '\''import importlib.metadata; importlib.metadata.distribution("pyprojectx")'\''
         if test "$3" = baseline; then
           exec "$TRAIL_VENV_PYTHON" -m pytest -q tests/unit/test_cli.py -k test_parse_args 1>&2
         fi
         exec "$TRAIL_VENV_PYTHON" -m pytest -q tests/unit/test_trail_qualification.py 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    cmake)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify cmake "$3"
         "$TRAIL_CMAKE" -S . -B "$TRAIL_CMAKE_BUILD_DIR" -DLEVELDB_BUILD_TESTS=OFF -DLEVELDB_BUILD_BENCHMARKS=OFF 1>&2
         "$TRAIL_CMAKE" --build "$TRAIL_CMAKE_BUILD_DIR" --target leveldb --parallel 2 1>&2
         hash_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/leveldb.dir/util/hash.cc.o" -print -quit)
         status_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/leveldb.dir/util/status.cc.o" -print -quit)
         test -n "$hash_object" && test -n "$status_object"
         shasum -a 256 "$hash_object" | awk "{print \$1}" > "$TRAIL_CMAKE_BUILD_DIR/trail-hash-before.sha256"
         shasum -a 256 "$status_object" | awk "{print \$1}" > "$TRAIL_CMAKE_BUILD_DIR/trail-status-before.sha256"' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    cmake-modern)
      run_json "precheck-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify cmake-modern "$3"
         "$TRAIL_CMAKE" --preset "$TRAIL_CMAKE_CONFIGURE_PRESET" -B "$TRAIL_CMAKE_MOUNTED_BUILD_DIR" -DCLI11_BUILD_TESTS=OFF -DCLI11_BUILD_EXAMPLES=ON 1>&2
         "$TRAIL_CMAKE" --build "$TRAIL_CMAKE_MOUNTED_BUILD_DIR" --target minimal --parallel 2 1>&2
         minimal_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/minimal.dir/minimal.cpp.o" -print -quit)
         precompile_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/CLI11.dir/Precompile.cpp.o" -print -quit)
         test -n "$minimal_object" && test -n "$precompile_object"
         shasum -a 256 "$minimal_object" | awk "{print \$1}" > "$TRAIL_CMAKE_BUILD_DIR/trail-minimal-before.sha256"
         shasum -a 256 "$precompile_object" | awk "{print \$1}" > "$TRAIL_CMAKE_BUILD_DIR/trail-precompile-before.sha256"
         if test "$3" != baseline; then
           strings "$TRAIL_CMAKE_BUILD_DIR/examples/minimal" | grep -F "$3" >/dev/null
           test "$("$TRAIL_CCACHE" --print-stats | awk "\$1 == \"direct_cache_hit\" {print \$2}")" -gt 0
         fi' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
  esac
}

run_framework_check() {
  local lane=$1
  local expected=${2:-$lane}
  local go_package
  case "$framework" in
    go|go-workspace)
      if [[ $framework == go ]]; then
        go_package=./version
      else
        go_package=./common
      fi
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify "$3" "$4"
         exec "$TRAIL_GO" test "$5" -run "^TestTrailQualificationMarker$" -count=1 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$framework" "$expected" \
        "$go_package"
      ;;
    yarn)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify yarn "$3"
         exec "$TRAIL_YARN" mocha test.js --grep "Trail qualification marker" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    bun)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify bun "$3"
         exec "$TRAIL_BUN" test tests/app.test.ts -t "Trail qualification marker" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    pnpm)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify pnpm "$3"
         "$TRAIL_PNPM" exec tsc --noEmit 1>&2
         "$TRAIL_PNPM" run build 1>&2
         exec "$TRAIL_PNPM" exec vitest run tests/http-helpers/index.test.ts -t "Trail qualification marker" 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    npm)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify npm "$3"
         "$TRAIL_NPM" run build -- --no-pack 1>&2
         exec "$TRAIL_NODE" --test --enable-source-maps dist-node/test/version.test.js 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    python)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify python "$3"
         "$TRAIL_VENV_PYTHON" -m compileall -q src/tap
         exec "$TRAIL_VENV_PYTHON" -m pytest -q tests/test_line.py -k trail_qualification_marker 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    uv)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify uv "$3"
         "$TRAIL_VENV_PYTHON" -c '\''import importlib.metadata; importlib.metadata.distribution("pyprojectx")'\''
         exec "$TRAIL_VENV_PYTHON" -m pytest -q tests/unit/test_trail_qualification.py 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    cmake)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify cmake "$3"
         rebuild_log="$TRAIL_CMAKE_BUILD_DIR/trail-rebuild.log"
         "$TRAIL_CMAKE" --build "$TRAIL_CMAKE_BUILD_DIR" --target leveldb --parallel 2 >"$rebuild_log" 2>&1
         cat "$rebuild_log" >&2
         grep "hash.cc.o" "$rebuild_log" >/dev/null
         if grep "status.cc.o" "$rebuild_log" >/dev/null; then
           echo "unaffected status.cc was recompiled" >&2
           exit 1
         fi
         hash_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/leveldb.dir/util/hash.cc.o" -print -quit)
         status_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/leveldb.dir/util/status.cc.o" -print -quit)
         hash_after=$(shasum -a 256 "$hash_object" | awk "{print \$1}")
         status_after=$(shasum -a 256 "$status_object" | awk "{print \$1}")
         test "$hash_after" != "$(cat "$TRAIL_CMAKE_BUILD_DIR/trail-hash-before.sha256")"
         test "$status_after" = "$(cat "$TRAIL_CMAKE_BUILD_DIR/trail-status-before.sha256")"
         cat > "$TRAIL_CMAKE_BUILD_DIR/trail-check.cc" <<EOF
#include <string>
#include "util/hash.h"
int main() { return std::string(leveldb::TrailQualificationMarker()) == "$3" ? 0 : 1; }
EOF
         c++ -std=c++11 -I. "$TRAIL_CMAKE_BUILD_DIR/trail-check.cc" "$TRAIL_CMAKE_BUILD_DIR/libleveldb.a" -pthread -o "$TRAIL_CMAKE_BUILD_DIR/trail-check"
         exec "$TRAIL_CMAKE_BUILD_DIR/trail-check"' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
    cmake-modern)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$1" "$2" verify cmake-modern "$3"
         rebuild_log="$TRAIL_CMAKE_BUILD_DIR/trail-rebuild.log"
         "$TRAIL_CMAKE" --build "$TRAIL_CMAKE_MOUNTED_BUILD_DIR" --target minimal --parallel 2 >"$rebuild_log" 2>&1
         cat "$rebuild_log" >&2
         grep "minimal.cpp.o" "$rebuild_log" >/dev/null
         if grep "Precompile.cpp.o" "$rebuild_log" >/dev/null; then
           echo "unaffected Precompile.cpp was recompiled" >&2
           exit 1
         fi
         minimal_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/minimal.dir/minimal.cpp.o" -print -quit)
         precompile_object=$(find "$TRAIL_CMAKE_BUILD_DIR" -path "*CMakeFiles/CLI11.dir/Precompile.cpp.o" -print -quit)
         test "$(shasum -a 256 "$minimal_object" | awk "{print \$1}")" != "$(cat "$TRAIL_CMAKE_BUILD_DIR/trail-minimal-before.sha256")"
         test "$(shasum -a 256 "$precompile_object" | awk "{print \$1}")" = "$(cat "$TRAIL_CMAKE_BUILD_DIR/trail-precompile-before.sha256")"
         strings "$TRAIL_CMAKE_BUILD_DIR/examples/minimal" | grep -F "$3" >/dev/null
         exec "$TRAIL_CMAKE_BUILD_DIR/examples/minimal" --help 1>&2' \
        trail "$python3_bin" "$SEMANTIC_EDITOR" "$expected"
      ;;
  esac
}

cd "$repository_root"
run_json init init --from-git
if [[ $framework == npm ]]; then
  "$TRAIL_BIN" --quiet ignore add 'dist-node/'
  "$TRAIL_BIN" --quiet ignore check 'dist-node/version.js'
fi

previous=
for lane in agent-a agent-b agent-c; do
  if [[ -z $previous ]]; then
    run_json "spawn-$lane" lane spawn "$lane" --from main --workdir-mode "$workdir_mode"
  else
    run_json "spawn-$lane" lane spawn "$lane" --from "$previous" --workdir-mode "$workdir_mode"
  fi
  case "$lane" in
    agent-a) expected=baseline ;;
    agent-b) expected=agent-a ;;
    agent-c) expected=agent-b ;;
  esac
  if [[ $lane == agent-a && -f $repository_root/uv.lock ]]; then
    uv_plan=$qualification_root/uv-prewarm-plan.json
    "$TRAIL_BIN" --format json env plan "$lane" --adapter python > "$uv_plan"
    uv_namespace=$(jq -er '.caches[] | select(.name == "python-downloads") | .namespace_id' "$uv_plan")
    uv_cache=$repository_root/.trail/cache/namespaces/$uv_namespace/uv
    mkdir -p "$uv_cache"
    UV_CACHE_DIR=$uv_cache \
      UV_PROJECT_ENVIRONMENT=$qualification_root/uv-prewarm-venv \
      UV_NO_PROGRESS=1 \
      uv sync --frozen --no-progress
  fi
  run_json "plan-$lane" env plan "$lane" --adapter "$component_selector"
  run_framework_precheck "$lane" "$expected"
  run_json "generation-before-edit-$lane" env generation "$lane"
  run_edit "$lane"
  run_json "sync-$lane" env sync component "$component_id" \
    --adapter "$component_selector" --lane "$lane"
  run_framework_check "$lane"
  run_json "generation-$lane" env generation "$lane"
  previous=$lane
done

if [[ $framework == yarn || $framework == bun ]]; then
  run_json spawn-invalidation lane spawn invalidation --from agent-c --workdir-mode "$workdir_mode"
  run_json generation-before-invalidation env generation invalidation
  run_json invalidation-edit lane exec invalidation -- \
    "$python3_bin" "$SEMANTIC_EDITOR" invalidate "$framework" agent-c
  run_json sync-invalidation env sync component "$component_id" \
    --adapter "$component_selector" --lane invalidation
  run_framework_check invalidation agent-c
  run_json generation-invalidation env generation invalidation
fi

python3 "$SCRIPT_DIR/check-real-framework-handoff.py" \
  "$TRAIL_FRAMEWORK_EVIDENCE_DIR" "$framework" "$repository" "$revision" "$component_id"

git diff --quiet -- || die "qualification mutated the Git checkout"
git diff --cached --quiet -- || die "qualification mutated the Git index"
echo "real-framework handoff evidence: $TRAIL_FRAMEWORK_EVIDENCE_DIR/evidence.json"
