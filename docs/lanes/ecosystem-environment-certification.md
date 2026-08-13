# Ecosystem environment certification

Trail distinguishes implementation support from certification evidence. Recognition or
successful planning is not enough to claim that a framework can perform a safe Agent
A → B → C handoff. Certification requires a pinned repository and tools, exact semantic
checkpoint ancestry, deterministic plans, real validation, identity-input invalidation,
private-output isolation, and hashes for every authoritative raw report.

## Current status

| Ecosystem variant | Trail contract | Local real-repository evidence | Hosted status |
| --- | --- | --- | --- |
| Go module and `go.work` multi-module | built-in `trail/go-vendor@1`/`@2` | qualified on macOS NFS-COW | opt-in matrix; not promoted to required CI |
| npm, pnpm, Yarn Classic, Bun | built-in `trail/node@1` | qualified on macOS NFS-COW | opt-in matrix; Yarn Berry/PnP remains unsupported |
| Python hash locks and `uv.lock` projects | built-in `trail/python-venv@1` | qualified on macOS NFS-COW | opt-in matrix; Poetry/PDM/Pipenv locks remain unsupported |
| CMake and modern CMake/Ninja/presets/ccache/vcpkg | built-in `trail/cmake-build@1` | qualified on macOS NFS-COW | opt-in matrix; Conan remains recognized but unsupported |
| Approved Node lifecycle/native addon | built-in Node approval contract | qualified on macOS NFS-COW with real denied-network/write checks | hosted promotion pending |
| Bazel, Gradle, Maven | protocol-v2 example plugin packages | qualified locally with pinned real repositories and offline construction | experimental packages; hosted promotion pending |
| Nix | protocol-v2 example plugin package plus external immutable identities | qualified locally with a pinned `NixOS/templates` revision and digest-pinned Linux/arm64 builder | experimental package; hosted promotion pending |

“Qualified locally” describes passing evidence for the named platform. It is not a
cross-platform claim. A row becomes hosted-certified only after its owning workflow is
green and required for the release; skipped or unavailable native backends are not
passing evidence.

## Canonical evidence

The external-system checker accepts exactly 26 raw JSON reports for a three-lane run:
the installed distribution and conformance result; spawn, checkpoint, repeated plan,
sync, and semantic validation for A/B/C; plus the same identity-authority invalidation
reports. It verifies raw hashes again when reading the sealed `evidence.json`:

```sh
python3 scripts/check-external-build-system-handoff.py \
  /path/to/certification-v1 nix owner/repository <revision> external-build.nix
python3 scripts/check-external-build-system-handoff.py \
  --verify /path/to/certification-v1
```

The built-in framework harness uses the equivalent
`scripts/check-real-framework-handoff.py` contract. Evidence directories are generated
qualification artifacts and are not committed to the Trail source tree.

## External package workflow

Build one shared example executable and package it with an ecosystem-specific manifest:

```sh
CARGO_TARGET_DIR=/path/out cargo build \
  -p trail-environment-adapter-sdk --example ecosystem-build-adapter --locked
TRAIL_ECOSYSTEM_ADAPTER_BIN=/path/out/debug/examples/ecosystem-build-adapter \
  scripts/build-ecosystem-adapter-package.sh nix /new/package-directory
trail env plugin inspect /new/package-directory --format json
trail env plugin install /new/package-directory --format json
```

The Bazel, Gradle, and Maven packages declare an offline process-tree action, host-owned
performance caches where applicable, and lane-private mutable output. The Nix package is
metadata-only: it runs no process and receives no cache, network, secret, Docker socket,
or host-store access. It requires a strict `trail.nix.toml` marker whose `flake.lock`
digest matches the pinned source and records:

- `locked = true` and `pure = true`;
- exact Nix version, digest-pinned builder image, and platform;
- package and check `/nix/store/...` references with NAR SHA-256 digests.

Trail records those provider-owned identities and creates only lane-private profile and
client-state directories. Qualification separately proves the reported paths using
`nix build --offline --no-write-lock-file --option pure-eval true`; changing lock bytes
invalidates the Trail component even when JSON meaning and Nix store results are
unchanged.

## Promotion rule

Do not promote a status because a synthetic fixture passes. Promotion requires the
common malicious-package suite, deterministic planning, exact distribution binding,
real-tool validation, native lane isolation, authority invalidation, and sealed raw
evidence. Record the tested repository revision, tool/image digest, operating system,
architecture, and layered backend. Keep unsupported variants fail-closed and name the
missing contract explicitly.
