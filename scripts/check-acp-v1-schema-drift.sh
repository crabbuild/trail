#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_manifest="$repo_root/trail/tests/fixtures/acp/v1/source.json"
fixture_dir="$repo_root/trail/tests/fixtures/acp/v1"
repository="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["repository"])' "$source_manifest")"
old_commit="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["commit"])' "$source_manifest")"
new_commit="${ACP_V1_UPSTREAM_REVISION:-$(git ls-remote "$repository.git" refs/heads/main | awk '{print $1}')}"
if [[ -z "$new_commit" ]]; then
  echo "unable to resolve the upstream ACP main revision" >&2
  exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
base="https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/$new_commit/schema/v1"
curl --fail --location --silent --show-error "$base/schema.json" --output "$tmp/schema.json"
curl --fail --location --silent --show-error "$base/meta.json" --output "$tmp/meta.json"

digest() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

old_schema="$(digest "$fixture_dir/schema.json")"
old_meta="$(digest "$fixture_dir/meta.json")"
new_schema="$(digest "$tmp/schema.json")"
new_meta="$(digest "$tmp/meta.json")"
printf 'ACP v1 pinned commit:   %s\n' "$old_commit"
printf 'ACP v1 upstream commit: %s\n' "$new_commit"
printf 'schema.json: %s -> %s\n' "$old_schema" "$new_schema"
printf 'meta.json:   %s -> %s\n' "$old_meta" "$new_meta"

if ! cmp -s "$fixture_dir/schema.json" "$tmp/schema.json" || ! cmp -s "$fixture_dir/meta.json" "$tmp/meta.json"; then
  echo "ACP v1 schema drift detected; review and repin the normative contract" >&2
  exit 1
fi
echo "ACP v1 schema has not drifted"
