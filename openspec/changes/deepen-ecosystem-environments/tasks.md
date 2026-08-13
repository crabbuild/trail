## 1. Contract and qualification foundations

- [x] 1.1 Add canonical ecosystem-certification evidence fields and adversarial verifier fixtures for tools, ancestry, identity, caches, outputs, invalidation, and raw hashes
- [x] 1.2 Extend the real-framework harness with manager/build-system-specific setup, semantic edits, validations, and stale-output assertions without weakening existing five-framework evidence
- [x] 1.3 Add CI matrix metadata and tool setup for the newly certified variants while keeping unstable real-repository jobs opt-in until promoted

## 2. Go multi-module workspaces

- [x] 2.1 Implement bounded, contained `go.work` member-graph parsing and discovery while preserving single-module compatibility
- [x] 2.2 Plan and construct graph-aware workspace vendor output with exact member/module/workspace/tool/platform identity and managed Go caches
- [x] 2.3 Add unit and adversarial tests for valid graphs, replacements, duplicates, traversal, symlinks, graph bounds, and deterministic ordering
- [x] 2.4 Run and seal a real multi-module Go A → B → C workspace qualification

## 3. Yarn and Bun certification

- [x] 3.1 Complete Yarn Classic frozen snapshot/install/cache handling and keep Berry/PnP explicitly fail-closed
- [x] 3.2 Complete Bun frozen snapshot/install/cache handling with exact manager and platform identity
- [x] 3.3 Add manager-specific unit/integration tests for source-only reuse, lock invalidation, cache/private-upper isolation, and unsupported layouts
- [x] 3.4 Run and seal pinned real-repository A → B → C qualifications for Yarn and Bun

## 4. Python uv project environments

- [x] 4.1 Replace `uv.lock` pip-style installation with contained `uv sync --frozen` project semantics in the physical private upper
- [x] 4.2 Include uv workspace/member/source authority and selected Python/uv identities in bounded deterministic planning
- [x] 4.3 Validate installed project/dependency state and add mismatch, escape, offline, interruption, reopen, and private-output tests
- [x] 4.4 Run and seal a real uv-locked Python A → B → C qualification

## 5. Modern CMake environments

- [x] 5.1 Add bounded contained CMake preset/include selection and selected configure-preset identity
- [x] 5.2 Add Ninja, compiler, toolchain-file, and generator identity plus direct lane-private build bindings
- [x] 5.3 Add host-scoped ccache declarations and verify cache-only sharing with private build-tree isolation
- [x] 5.4 Add pinned vcpkg manifest/baseline/toolchain planning, offline cache/output policy, and escape rejection
- [x] 5.5 Add unit, adversarial, interruption, reopen, and real-tool tests for presets, Ninja, ccache, toolchains, and vcpkg
- [x] 5.6 Run and seal a real modern-CMake A → B → C qualification

## 6. Approved Node lifecycle scripts and native addons

- [x] 6.1 Define and parse one canonical committed versioned approval policy with exact manager, lock, package/script selectors, capabilities, and output declarations
- [x] 6.2 Bind approval provenance, ABI/platform/compiler identities, network policy, and output policy into component identity and receipts
- [x] 6.3 Enforce allowlisted lifecycle execution in the native sandbox and reject unapproved scripts, undeclared writes, network, secrets, and missing toolchains
- [x] 6.4 Add native-addon real-tool tests plus malicious transitive-package, interruption, recovery, redaction, and cross-lane isolation tests
- [x] 6.5 Run and seal a pinned native-addon A → B → C qualification

## 7. Bazel adapter/plugin certification

- [x] 7.1 Author a versioned Bazel recipe/package declaring module/lock/tool identity, cache-only repository state, private output bases, and validations
- [x] 7.2 Pass SDK/common malicious-plan, determinism, containment, recovery, and redaction conformance for Bazel
- [x] 7.3 Run and seal a pinned Bazel A → B → C real-repository qualification

## 8. Gradle adapter/plugin certification

- [x] 8.1 Author a versioned Gradle recipe/package with verified wrapper/tool, lock/catalog/settings identity, bounded caches, private build state, and daemon policy
- [x] 8.2 Pass SDK/common malicious-plan, determinism, containment, recovery, and redaction conformance for Gradle
- [x] 8.3 Run and seal a pinned Gradle A → B → C real-repository qualification

## 9. Maven adapter/plugin certification

- [x] 9.1 Author a versioned Maven recipe/package with verified wrapper/tool, POM/reproducible dependency authority, secret-free settings, bounded cache, and private target state
- [x] 9.2 Pass SDK/common malicious-plan, determinism, containment, recovery, and redaction conformance for Maven
- [x] 9.3 Run and seal a pinned offline Maven A → B → C real-repository qualification

## 10. Nix adapter/plugin certification

- [x] 10.1 Author a versioned Nix recipe/package requiring pinned pure evaluation and representing store paths as external immutable identities
- [x] 10.2 Pass SDK/common malicious-plan, determinism, containment, recovery, redaction, and impure/unlocked rejection conformance for Nix
- [x] 10.3 Run and seal a pinned Nix flake A → B → C real-repository qualification

## 11. Public contracts and release evidence

- [x] 11.1 Align Rust, CLI JSON, HTTP/OpenAPI, MCP, and SDK reports for any new approval/certification fields and add compatibility tests
- [x] 11.2 Update README, adapter/environment design, lane guides, reference docs, security guidance, and changelog with certified versus recognized platform status
- [x] 11.3 Run formatting, workspace check/test/Clippy, adapter SDK, environment lifecycle, native backend, real-tool, and hosted certification gates
- [x] 11.4 Audit every spec scenario against authoritative evidence and leave no variant labeled certified without a passing platform gate
