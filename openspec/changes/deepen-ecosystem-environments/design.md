## Context

Trail already has a framework-neutral environment graph and built-in adapters for Go, Node, Python, and CMake. Its host owns identity, isolated construction, shared caches, private outputs, generation activation, recovery, and managed command bindings. The remaining ecosystem gaps are not all the same kind: some need richer built-in graph discovery, some need an explicit trust policy for repository code, and Bazel/Gradle/Maven/Nix should prove the extension contract rather than grow four special cases in core.

The implementation must preserve source immutability, deterministic planning, bounded inputs and subprocess output, no implicit network or secrets, atomic activation, lane-private mutable state, and exact A → B → C ancestry. Existing environment wire types and schema versions should remain additive unless evidence proves a new stored meaning is unavoidable.

## Goals / Non-Goals

**Goals:**

- Certify common variants already recognized by built-in adapters: Go workspaces, Yarn, Bun, fully installed uv environments, and modern CMake workflows.
- Make repository-code execution explicit, inspectable, and deny-by-default for Node lifecycle scripts and native addons.
- Exercise Bazel, Gradle, Maven, and Nix through repository recipes or signed plugins with the same host-owned artifact lifecycle.
- Produce reproducible local and hosted evidence that successor lanes reuse only correctness-compatible state.
- Keep all public reports and documentation explicit about shared, private, cache-only, and external content.

**Non-Goals:**

- Claim arbitrary ecosystem build scripts are hermetic or deterministic.
- Share writable compiler/build directories between lanes.
- Add credentials or open network access to default built-in resolution.
- Implement every package manager feature, remote execution service, or lockfile dialect.
- Add one-off Bazel, Gradle, Maven, or Nix execution paths outside the adapter/recipe boundary.

## Decisions

### 1. Go workspaces are one graph-aware component

A directory containing `go.work` becomes one `trail/go-vendor@2` component. Trail parses the bounded `use` graph, normalizes and contains every module path, rejects duplicates, replacements or module paths that escape the component root, and includes `go.work`, optional `go.work.sum`, every member `go.mod`/`go.sum`, the complete pinned source root, Go executable identity, platform, and policy in its identity. Construction uses argv-only `go work vendor` against the pinned source projection and managed Go caches. Single-module repositories remain on the v1-compatible contract.

Alternative: emit one component per module. Rejected because `go work vendor`, workspace replacements, and cross-module dependencies have graph-wide semantics and atomic output.

### 2. Yarn and Bun graduate through the existing Node adapter

Yarn Classic and Bun retain manager-specific snapshot formats, frozen argv, cache namespaces, and ordinary `node_modules` immutable-seed/private-upper behavior. Yarn Berry/PnP remains fail-closed until a separate PnP binding contract exists. Real-repository gates must test both a source-only successor and an invalidating lock/policy change; unit fixtures alone are insufficient certification.

Alternative: certify only synthetic repositories. Rejected because manager wrappers, lock formats, and platform behavior are exactly what the qualification must prove.

### 3. `uv.lock` is installed with project semantics

For a project with `uv.lock`, Trail runs `uv sync --frozen --offline --no-progress` in the physical lane-private candidate upper, with `UV_PROJECT_ENVIRONMENT` pointing at the candidate `.venv` and a shared performance-only uv cache. The lockfile, `pyproject.toml`, workspace/member metadata selected by uv, Python identity, uv identity, platform, and policy determine the component key. Hash-pinned requirements keep their separate `uv pip install --require-hashes` contract. A successful plan must validate that the environment contains the locked project distribution/dependencies rather than only a Python executable.

Alternative: export `uv.lock` to requirements and install with pip semantics. Rejected because it loses uv workspace, source, group, and project-install meaning.

### 4. CMake configuration is modeled, build output remains private

The adapter discovers `CMakePresets.json` and optional `CMakeUserPresets.json` only when all includes remain inside the pinned repository and the selected configure preset is unambiguous or explicitly configured. It fingerprints selected preset expansion, generator, toolchain file bytes, CMake/Ninja/compiler identities, platform, and dependency-manager identity. It binds a lane-private build tree, `CMAKE_BUILD_PARALLEL_LEVEL` policy, and an optional host-scoped ccache namespace. The first dependency-manager certification uses vcpkg manifest mode with a pinned baseline and explicit toolchain identity; registries/downloads are cache-only and require the qualification job's prewarmed/offline inputs. Conan remains visible but unsupported until an equivalent lock/profile contract lands.

Alternative: publish CMake build trees as immutable layers. Rejected because CMake embeds absolute source/build paths and generators mutate the tree incrementally.

### 5. Node repository code requires a versioned approval

Default Node construction continues to disable lifecycle scripts. A repository may request scripts/native addons only through a committed Trail policy containing an exact package-manager, lock digest, approved package/script selectors, platform/toolchain compatibility, network denial, and output policy. Trail records the policy digest and approval provenance in the component identity and receipt. Script-enabled installs never use the public script-disabled portability contract; native-addon results are ABI/platform/toolchain scoped, and unclassified writable outputs remain lane-private. Any script outside the allowlist, attempted network access, undeclared write, or missing compiler identity fails closed.

Alternative: pass npm/Yarn/Bun's ordinary “enable scripts” flag. Rejected because it delegates authority to transitive repository code without an auditable scope.

### 6. External build systems certify the extension ladder

Bazel, Gradle, Maven, and Nix each ship a versioned example recipe or adapter package plus a conformance manifest. Plans declare exact lock/config/tool inputs, host-managed executable identities, bounded caches, private outputs, denied network during construction, validation commands, and portability. Nix store paths are external immutable identities, not copied Trail layers. Certification runs the common malicious-plan suite, platform-appropriate real-tool construction, reopen/reuse, stale-input rejection, and A → B → C handoff verifier.

Alternative: add four built-in adapters. Rejected until the extension contract proves unable to express a required capability.

### 7. Certification is a first-class checked artifact

The qualification harness emits a canonical evidence document naming repository/revision, adapter/package digest, tool identities, platform/backend, lane ancestry, roots, keys, cache namespaces, outputs, executed validations, stale-input cases, and raw evidence hashes. Hosted CI validates the document with adversarial tests. Documentation labels a variant certified only when its required platform jobs pass.

## Risks / Trade-offs

- **Repository scripts execute untrusted code** → Keep scripts deny-by-default, require exact committed approval, apply native sandbox/network denial, and scope outputs/platform identity conservatively.
- **Go/CMake/Python workspace discovery can escape through includes or relative members** → Normalize against the pinned component root, reject symlink/traversal/absolute escapes, cap graph size and file bytes, and test hostile graphs.
- **Offline real-tool gates can be flaky without caches** → Pin repository revisions and tool versions, explicitly prewarm cache inputs in setup, run construction offline, and distinguish setup network from adapter authority.
- **CMake presets vary across generators and hosts** → Certify Ninja first, persist exact selected preset/generator/toolchain identity, and fail on ambiguity rather than selecting silently.
- **Private outputs reduce byte reuse** → Reuse correctness-neutral download/compiler caches and compatible private seeds only where the tool contract permits; report the trade-off rather than manufacturing a shared artifact.
- **External recipes may expose missing SDK capabilities** → Extend the common protocol additively with typed declarations only after a conformance test proves the gap; do not bypass host ownership.
- **Qualification matrix cost grows sharply** → Separate fast contract/adversarial jobs from scheduled or opt-in real-repository jobs, then promote stable gates to required CI per variant.

## Migration Plan

1. Land additive adapter behavior and fixtures behind existing fail-closed discovery states.
2. Add local real-tool tests and canonical evidence verifiers.
3. Add pinned hosted matrix entries as non-required qualification jobs.
4. Promote a variant's documentation from recognized/experimental to certified only after its platform evidence is green.
5. Preserve existing v1 single-module Go, script-disabled Node, hashed-requirements Python, and basic CMake behavior as compatible fallbacks.
6. Roll back by disabling the affected certification/adapter version; prior environment generations remain inspectable and no durable object is rewritten.

## Open Questions

- Whether Conan should follow vcpkg in this change after vcpkg certification, or remain the documented next C/C++ dependency-manager contract.
- Which Bazel/Gradle/Maven/Nix repositories provide small, stable, license-compatible hosted fixtures across the supported platforms.
- Whether approved Node lifecycle policy fits the existing repository environment document or merits a narrowly scoped dedicated policy file; implementation must choose one canonical source before public release.
