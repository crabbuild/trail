## Why

Trail's universal environment model recognizes more ecosystem shapes than its production evidence currently certifies. The remaining gaps force common monorepos, alternate package managers, native dependencies, and non-Rust build systems either to fail closed or to fall back to unqualified host behavior, undermining reliable A → B → C lane handoffs.

## What Changes

- Extend the built-in Go adapter from one module to graph-aware `go.work` multi-module workspaces.
- Add real-repository A → B → C certification for Yarn and Bun using their exact frozen-install and cache contracts.
- Install Python dependencies from `uv.lock` with `uv sync --frozen`, preserving a lane-private virtual environment and shared download cache.
- Expand the CMake adapter to model presets, Ninja, toolchain files, ccache, and one pinned C/C++ dependency-manager contract (vcpkg or Conan).
- Add a deny-by-default approval contract for Node lifecycle scripts and native addons, with platform/toolchain-sensitive identity and non-shareable handling where correctness cannot be proven.
- Certify Bazel, Gradle, Maven, and Nix through the existing repository recipe or external adapter/plugin contract rather than adding ad hoc execution paths.
- Require unit, integration, adversarial, real-tool, and real-repository evidence plus aligned CLI reports and public documentation for every newly certified variant.

## Capabilities

### New Capabilities

- `deep-ecosystem-environments`: Built-in Go, Node, Python, and CMake environment variants, their safety and reuse contracts, and semantic A → B → C qualification.
- `external-build-system-certification`: Conformance and real-tool certification for Bazel, Gradle, Maven, and Nix through the adapter/plugin contract.

### Modified Capabilities

None. The repository has no promoted main OpenSpec capability specs; this change codifies previously documented partial behavior as new enforceable requirements.

## Impact

- Built-in environment adapters under `trail/src/db/lane/workspace_{go,node,python,cmake}.rs` and shared planning, sandbox, cache, output, and generation code.
- Adapter SDK/package contracts and repository command recipes where external systems need richer declarations.
- CLI/Rust/HTTP/MCP environment reports if new policy, approval, graph, or certification evidence becomes public.
- Real-framework qualification scripts and the layered-workspaces/CI matrices on Linux, macOS, and Windows where supported.
- Environment design, adapter contract, lane workflow, security guidance, README, and changelog documentation.
- Host tool prerequisites for Go, Yarn, Bun, uv, CMake/Ninja/ccache, the selected C/C++ dependency manager, Bazel, Gradle, Maven, and Nix qualification jobs.
