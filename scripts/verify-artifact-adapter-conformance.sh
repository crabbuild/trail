#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

: "${CARGO_TARGET_DIR:?set a checkout-specific CARGO_TARGET_DIR beneath /Volumes/Workspace/crabbuild-target}"
if [[ "${GITHUB_ACTIONS:-}" != "true" && "${CARGO_TARGET_DIR}" != /Volumes/Workspace/crabbuild-target/* ]]; then
  printf '%s\n' 'CARGO_TARGET_DIR must be beneath /Volumes/Workspace/crabbuild-target' >&2
  exit 2
fi

output="${1:-}"
evidence_dir="$(mktemp -d)"
cleanup() {
  rm -rf -- "${evidence_dir}"
}
trap cleanup EXIT
report="${evidence_dir}/artifact-adapter-certification.json"

TRAIL_ARTIFACT_CERTIFICATION_OUTPUT="${report}" \
  cargo test -p trail --lib \
  db::lane::workspace_layer::tests::builtins_plugins_and_repository_v2_share_artifact_pipeline_conformance \
  --locked -- --exact >&2

python3 - "${report}" "${output}" <<'PY'
import json
import os
import pathlib
import sys
import tempfile

source = pathlib.Path(sys.argv[1])
destination = sys.argv[2]
reports = json.loads(source.read_text(encoding="utf-8"))
families = [report.get("producer_family") for report in reports]
if families != ["builtin", "plugin_v3", "repository_v2"]:
    raise SystemExit(f"unexpected producer families: {families!r}")
expected_stages = [
    "discovery", "resolution", "identity", "validation", "sealing", "cow",
    "recovery", "invalidation", "export", "retirement", "collection",
]
for report in reports:
    if report.get("schema") != "trail.artifact-adapter-certification/v1":
        raise SystemExit("unsupported certification report schema")
    if report.get("status") != "passed" or report.get("authority_effect") != "evidence_only":
        raise SystemExit("certification report did not pass as evidence-only")
    checks = report.get("checks")
    if not isinstance(checks, list) or [check.get("stage") for check in checks] != expected_stages:
        raise SystemExit("certification report stages are incomplete or unordered")
    if any(check.get("status") != "passed" or not check.get("applicable") for check in checks):
        raise SystemExit("required certification stage was skipped or failed")

encoded = json.dumps(reports, indent=2, sort_keys=True) + "\n"
if destination:
    target = pathlib.Path(destination)
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{target.name}.", dir=target.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, target)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise
else:
    sys.stdout.write(encoded)
PY
