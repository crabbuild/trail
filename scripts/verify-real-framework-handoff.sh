#!/usr/bin/env bash
# Qualify one pinned real repository through an Agent A -> B -> C native-COW
# handoff. This is an opt-in release evidence gate, not a networked unit test.
set -euo pipefail
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

die() {
  echo "verify-real-framework-handoff: $*" >&2
  exit 64
}

[[ $# == 1 ]] || die "usage: $0 <go|pnpm|npm|python|cmake>"
framework=$1
: "${TRAIL_BIN:?set TRAIL_BIN to the candidate Trail executable}"
: "${TRAIL_FRAMEWORK_EVIDENCE_DIR:?set TRAIL_FRAMEWORK_EVIDENCE_DIR to a new output directory}"
[[ $TRAIL_BIN == /* ]] || die "TRAIL_BIN must be absolute"
[[ -x $TRAIL_BIN ]] || die "TRAIL_BIN is not executable: $TRAIL_BIN"
[[ $TRAIL_FRAMEWORK_EVIDENCE_DIR == /* ]] || die "TRAIL_FRAMEWORK_EVIDENCE_DIR must be absolute"
[[ ! -e $TRAIL_FRAMEWORK_EVIDENCE_DIR ]] || die "evidence directory already exists"
qualification_root=${TRAIL_FRAMEWORK_WORK_ROOT:-$TRAIL_FRAMEWORK_EVIDENCE_DIR.work}
[[ $qualification_root == /* ]] || die "TRAIL_FRAMEWORK_WORK_ROOT must be absolute when set"
[[ ! -e $qualification_root ]] || die "qualification work directory already exists"

case "$framework" in
  go)
    repository=https://github.com/etcd-io/bbolt.git
    revision=55cb34b031c9855defb6c52db560a610f85bf5c3
    component_selector=go-vendor
    component_id=go-vendor
    ;;
  pnpm)
    repository=https://github.com/date-fns/date-fns.git
    revision=4098115cf705e3af7f663d8e5b0686e39a9f478a
    component_selector=node
    component_id=node
    ;;
  npm)
    repository=https://github.com/lodash/lodash.git
    revision=a666ba591064c8011988275790ad7d625279f09c
    component_selector=node
    component_id=node
    ;;
  python)
    repository=https://github.com/encode/httpx.git
    revision=b5addb64f0161ff6bfe94c124ef76f6a1fba5254
    component_selector=python
    component_id=python-venv
    ;;
  cmake)
    repository=https://github.com/google/leveldb.git
    revision=7ee830d02b623e8ffe0b95d59a74db1e58da04c5
    component_selector=cmake-build
    component_id=cmake-build
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
if [[ $framework == cmake ]]; then
  git -C "$repository_root" submodule update -q --init --recursive --depth=1
fi

run_json() {
  local output=$1
  shift
  "$TRAIL_BIN" --format json "$@" >"$TRAIL_FRAMEWORK_EVIDENCE_DIR/raw/$output.json"
}

run_edit() {
  local lane=$1
  run_json "edit-$lane" lane exec "$lane" -- /bin/sh -c \
    'printf "\nTrail real-framework qualification %s.\n" "$1" >> README.md' \
    trail "$lane"
}

run_framework_check() {
  local lane=$1
  case "$framework" in
    go)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'exec "$TRAIL_GO" test ./... -run "^TestTxStats_add$"'
      ;;
    pnpm)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'exec "$TRAIL_NODE" "$TRAIL_NODE_MODULES/vitest/vitest.mjs" run pkgs/core/src/getYear/test.ts'
      ;;
    npm)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'exec "$TRAIL_NODE" -e '\''const lodash = require("./lodash.js"); if (lodash.chunk([1,2,3], 2).length !== 2) process.exit(1)'\'''
      ;;
    python)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        '"$TRAIL_VENV_PYTHON" -c '\''import os,sys; assert os.path.realpath(sys.prefix) == os.path.realpath(os.environ["VIRTUAL_ENV"])'\'' && exec "$TRAIL_VENV_PYTHON" -m compileall -q httpx'
      ;;
    cmake)
      run_json "check-$lane" lane exec "$lane" -- /bin/sh -c \
        'set -eu
         "$TRAIL_CMAKE" -S . -B "$TRAIL_CMAKE_BUILD_DIR" -DLEVELDB_BUILD_TESTS=ON -DLEVELDB_BUILD_BENCHMARKS=OFF
         "$TRAIL_CMAKE" --build "$TRAIL_CMAKE_BUILD_DIR" --target leveldb_tests --parallel 2
         cmake_bin=${TRAIL_CMAKE%/*}
         exec "$cmake_bin/ctest" --test-dir "$TRAIL_CMAKE_BUILD_DIR" -R "^leveldb_tests$" --output-on-failure'
      ;;
  esac
}

cd "$repository_root"
run_json init init --from-git

previous=
for lane in agent-a agent-b agent-c; do
  if [[ -z $previous ]]; then
    run_json "spawn-$lane" lane spawn "$lane" --from main --workdir-mode nfs-cow
  else
    run_json "spawn-$lane" lane spawn "$lane" --from "$previous" --workdir-mode nfs-cow
    if [[ $framework == go || $framework == pnpm || $framework == npm ]]; then
      run_json "generation-before-edit-$lane" env generation "$lane"
    fi
  fi
  run_edit "$lane"
  run_json "sync-$lane" env sync component "$component_selector" --lane "$lane"
  run_framework_check "$lane"
  run_json "generation-$lane" env generation "$lane"
  previous=$lane
done

python3 "$SCRIPT_DIR/check-real-framework-handoff.py" \
  "$TRAIL_FRAMEWORK_EVIDENCE_DIR" "$framework" "$repository" "$revision" "$component_id"

git diff --quiet -- || die "qualification mutated the Git checkout"
git diff --cached --quiet -- || die "qualification mutated the Git index"
echo "real-framework handoff evidence: $TRAIL_FRAMEWORK_EVIDENCE_DIR/evidence.json"
