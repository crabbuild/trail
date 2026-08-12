#!/usr/bin/env bash
set -euo pipefail

die() {
  echo "build-ecosystem-adapter-package: $*" >&2
  exit 64
}

[[ $# == 2 ]] || die "usage: $0 <bazel|gradle|maven|nix> <new-output-directory>"
ecosystem=$1
output=$2
case "$ecosystem" in
  bazel|gradle|maven|nix) ;;
  *) die "unsupported ecosystem: $ecosystem" ;;
esac
[[ $output == /* ]] || die "output directory must be absolute"
[[ ! -e $output ]] || die "output directory already exists: $output"
: "${TRAIL_ECOSYSTEM_ADAPTER_BIN:?set TRAIL_ECOSYSTEM_ADAPTER_BIN to the built example executable}"
[[ $TRAIL_ECOSYSTEM_ADAPTER_BIN == /* ]] || die "TRAIL_ECOSYSTEM_ADAPTER_BIN must be absolute"
[[ -x $TRAIL_ECOSYSTEM_ADAPTER_BIN ]] || die "adapter executable is not runnable"

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repository_root=$(cd -- "$script_dir/.." && pwd)
template=$repository_root/tools/environment-adapters/$ecosystem/trail-adapter.toml.in
[[ -f $template ]] || die "package template is missing: $template"

mkdir -p "$output"
cp "$TRAIL_ECOSYSTEM_ADAPTER_BIN" "$output/ecosystem-build-adapter"
chmod 755 "$output/ecosystem-build-adapter"
cp "$template" "$output/trail-adapter.toml"
digest=$(shasum -a 256 "$output/ecosystem-build-adapter" | awk '{print $1}')
sed -i.bak "s/@EXECUTABLE_SHA256@/$digest/g" "$output/trail-adapter.toml"
unlink "$output/trail-adapter.toml.bak"
grep -F "sha256:$digest" "$output/trail-adapter.toml" >/dev/null
echo "$output"
