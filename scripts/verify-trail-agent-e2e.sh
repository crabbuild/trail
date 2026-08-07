#!/usr/bin/env bash
# End-to-end verification for Trail schema v1, agent lanes, Git handoff, and
# backup/restore. The disposable roots intentionally stay short because the
# macOS changed-path Unix socket is subject to SUN_LEN.
set -euo pipefail

trail_repo=${TRAIL_REPO:-/Users/haipingfu/Github/Trail}
trail_bin=${TRAIL_BIN:-/Volumes/Workspace/crabbuild-target/trail-e2e-v1/release/trail}
trail_source_repo=${TRAIL_SOURCE_REPO:-/Volumes/Workspace/Github/django-compass}
trail_target_dir=${TRAIL_TARGET_DIR:-/Volumes/Workspace/crabbuild-target/trail-e2e-v1}
trail_tag=${TRAIL_E2E_TAG:-tve2e}
external_root=${TRAIL_EXTERNAL_ROOT:-/Volumes/Workspace/Github/crabbuild}
backup_root=${TRAIL_BACKUP_ROOT:-/Volumes/Workspace/trail-e2e-backups}

workspace_volume=/Volumes/Workspace
agent_root="$external_root/$trail_tag"
backup_source="$external_root/${trail_tag}s"
restore_root="$external_root/${trail_tag}r"
reject_root="$external_root/${trail_tag}j"
backup_path="$backup_root/${trail_tag}-clean.tar"

say() {
  printf '\n== %s ==\n' "$1"
}

run() {
  printf '\n$'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

if [[ ! -d "$workspace_volume" || ! -w "$workspace_volume" ]]; then
  printf 'error: %s is unavailable or not writable; refusing to use local disk\n' \
    "$workspace_volume" >&2
  exit 2
fi
if [[ ! -d "$trail_source_repo/.git" ]]; then
  printf 'error: source repository is missing: %s\n' "$trail_source_repo" >&2
  exit 2
fi
if [[ ! -d "$trail_repo/.git" ]]; then
  printf 'error: Trail checkout is missing: %s\n' "$trail_repo" >&2
  exit 2
fi

mkdir -p "$trail_target_dir" "$external_root" "$backup_root"
if [[ ! -w "$trail_target_dir" ]]; then
  printf 'error: target directory is not writable: %s\n' "$trail_target_dir" >&2
  exit 2
fi

for path in "$agent_root" "$backup_source" "$restore_root" "$reject_root" "$backup_path"; do
  if [[ -e "$path" ]]; then
    printf 'error: refusing to overwrite existing disposable path: %s\n' "$path" >&2
    printf '       choose another TRAIL_E2E_TAG (for example tve2f)\n' >&2
    exit 2
  fi
done

say "Build and identify the release binary"
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

source_head=$(git -C "$trail_source_repo" rev-parse HEAD)

say "Initialize a real-repository clone and create a clean backup"
run git clone --local --no-hardlinks "$trail_source_repo" "$backup_source"
run "$trail_bin" --workspace "$backup_source" init --from-git --branch main --format json
run "$trail_bin" --workspace "$backup_source" status --format json
run "$trail_bin" --workspace "$backup_source" doctor --format json
run "$trail_bin" --workspace "$backup_source" fsck --format json
run "$trail_bin" --workspace "$backup_source" backup create "$backup_path" --format json
run "$trail_bin" --workspace "$backup_source" backup verify "$backup_path" --format json

say "Restore the backup into a second disposable clone"
run git clone --local --no-hardlinks "$trail_source_repo" "$restore_root"
run "$trail_bin" --workspace "$restore_root" backup restore "$backup_path" --force --format json
run "$trail_bin" --workspace "$restore_root" status --format json
run "$trail_bin" --workspace "$restore_root" doctor --format json
run "$trail_bin" --workspace "$restore_root" fsck --format json

say "Run a deterministic Claude Code-provider agent task"
run git clone --local --no-hardlinks "$trail_source_repo" "$agent_root"
run "$trail_bin" --workspace "$agent_root" init --from-git --branch main --format json
run "$trail_bin" --workspace "$agent_root" agent guide --format json
run "$trail_bin" --workspace "$agent_root" agent start \
  --provider claude-code \
  --name trail-e2e-agent \
  --workdir-mode auto \
  --format json -- \
  /bin/sh -lc 'set -eu; printf "schema v1\\n" > trail-agent-e2e.txt'
run "$trail_bin" --workspace "$agent_root" agent test latest \
  --suite smoke --format json -- \
  /bin/sh -lc 'set -eu; test -f trail-agent-e2e.txt; test "$(tail -n 1 trail-agent-e2e.txt)" = "schema v1"'
run "$trail_bin" --workspace "$agent_root" agent eval latest \
  --suite quality --score 1 --threshold 0.5 --format json -- \
  /bin/sh -lc 'set -eu; test -s trail-agent-e2e.txt'
run "$trail_bin" --workspace "$agent_root" agent validate latest --format json
run "$trail_bin" --workspace "$agent_root" agent mark-reviewed latest --format json
run "$trail_bin" --workspace "$agent_root" agent ready latest --format json
run "$trail_bin" --workspace "$agent_root" agent apply latest --dry-run --format json
run "$trail_bin" --workspace "$agent_root" agent apply latest \
  --into-current-git-branch \
  --message "Apply agent task: Trail schema v1 E2E" \
  --format json

[[ "$(git -C "$agent_root" show HEAD:trail-agent-e2e.txt)" == "schema v1" ]]
[[ -z "$(git -C "$agent_root" status --short --untracked-files=no)" ]]
run "$trail_bin" --workspace "$agent_root" index reconcile --format json
agent_status=$("$trail_bin" --workspace "$agent_root" status --format json)
printf '%s\n' "$agent_status"
printf '%s\n' "$agent_status" | grep -Eq '"worktree_state"[[:space:]]*:[[:space:]]*"Clean"'
run "$trail_bin" --workspace "$agent_root" doctor --format json
run "$trail_bin" --workspace "$agent_root" fsck --format json
printf 'agent_git_handoff=passed\n'

say "Reject a non-v1 schema and reinitialize explicitly"
run git clone --local --no-hardlinks "$trail_source_repo" "$reject_root"
run "$trail_bin" --workspace "$reject_root" init --from-git --branch main --format json
run sqlite3 "$reject_root/.trail/index/trail.sqlite" \
  "PRAGMA user_version=2; UPDATE schema_meta SET value='2',updated_at=strftime('%s','now') WHERE key='schema.version';"
set +e
reject_output=$("$trail_bin" --workspace "$reject_root" status --format json 2>&1)
reject_rc=$?
set -e
printf '%s\n' "$reject_output"
[[ "$reject_rc" -ne 0 ]]
printf '%s\n' "$reject_output" | grep -q 'SCHEMA_REINITIALIZE_REQUIRED'
run "$trail_bin" --workspace "$reject_root" init --force --from-git --branch main --format json
run "$trail_bin" --workspace "$reject_root" doctor --format json
run "$trail_bin" --workspace "$reject_root" fsck --format json
printf 'schema_rejection_and_reinit=passed\n'

[[ "$(git -C "$trail_source_repo" rev-parse HEAD)" == "$source_head" ]]
printf 'source_checkout_unchanged=passed\n'
printf '\nALL TRAIL END-TO-END CHECKS PASSED\n'
