## Context

`optimize-large-repo-lanes` is now implemented on the current Trail branch. This follow-up therefore starts from shipped behavior, not from the earlier design assumption that output policy, COW inheritance, promotion, singleflight, lazy layer access, or user-defined command recipes still need to be invented.

### Audited baseline

| Shipped foundation | Current ownership/evidence | Consequence for this change |
| --- | --- | --- |
| `immutable_shared`, `immutable_seed_private`, `writable_private`, and `disposable` output policies | `EnvironmentOutputPolicy`, SDK `AdapterOutputPolicy`, schema output rows | Preserve these values and extend their backing identity/storage; do not replace the policy model. |
| Reuse modes, sharing scopes, and `never`/`manual`/`on_sync`/`successful_gate` publication | report models, SDK, recipes, generation outputs | Reuse existing policy fields and promotion authorization. |
| Typed component DAG and `build_requires`, `invalidates_with`, `runtime_requires`, `binds_after` edges | environment planner, command recipes, protocol v2 | Extend key inputs and discovery outcomes; do not add another graph scheduler. |
| Bounded parallel preparation and per-key singleflight | `workspace_environment.rs`, `workspace_layer.rs` | Upgrade ownership and evidence for content sealing; keep scheduling semantics. |
| Journaled promotion into a successor generation | `workspace_layer_publications` and `env promote` | Reuse the publication fence, quiescence, and activation transaction. |
| Immutable lower plus fresh private COW state, component-granular inheritance | workspace layers/views and native NFS/FUSE/Dokan tests | Change only how immutable lower content is stored/materialized. |
| Paged layer manifests, verification stamps, prefetch, quota, GC, and lane space | `workspace_layer.rs`, cache CLI/reports | Make these CAS-aware; retain the operations and reports. |
| `trail.environment.toml` v1 includes/profiles and restricted `trail/command@1` components | `workspace_recipe.rs`, adapter documentation | Add a v2 schema in the same file; do not create `trail.artifacts.toml`. |
| Adapter protocols v1/v2 and signed package trust/revocation | adapter SDK and plugin host | Add explicit v3 negotiation and conversion. |
| CLI/HTTP/MCP/Rust parity for environment operations | shared lane reports and interface tests | Extend shared models; do not implement transport-specific domain behavior. |

### Remaining gaps proven by the audit

1. Cargo discovery requires both `Cargo.toml` and `Cargo.lock`; Node discovery similarly requires `package.json` plus a supported lock. A manifest-only component disappears instead of reporting resolution state.
2. `WorkspaceLayerKeyV1` describes desired inputs, and `layer_id` is derived from its digest. Produced byte identity is not independent from the request identity.
3. Publication copies a complete output directory to `.trail/cache/layers/<layer-id>`. Manifest pages store file hashes, but file bytes are authoritative only in that copied tree. Two desired keys producing equal files therefore duplicate those bytes.
4. The current layer manifest includes desired-key and publication fields such as layer ID and creation time, so it cannot serve as a pure deterministic content-tree identity.
5. There is no durable resolution snapshot, artifact nondeterminism quarantine, or artifact-specific attestation.
6. `trail.environment/v1` command recipes are intentionally restricted to the shipped subset. They cannot express controlled resolution, typed validations, several action phases, or explicit generated-source export.

The product boundary remains: adapters understand frameworks; Trail understands artifact lifecycle. Git remains the publication system for source. `.trail/` contains private operational state. Adapters plan but do not execute, mount, publish, update SQLite, resolve raw secrets, or collect storage.

### Deliverables

| Deliverable | Increment beyond the merged baseline | Acceptance evidence |
| --- | --- | --- |
| Visible incomplete discovery | Proposal statuses and recovery actions without executing tools | Cargo/Node manifest-only fixtures and side-effect assertions |
| Resolution snapshots | Pinned, content-addressed environment metadata with controlled authority | resolver success/failure/reuse/refresh/recovery tests |
| Identity v2 | separate desired, tree-content, artifact, and binding identities | canonicalization, invalidation, divergent-output, reopen tests |
| Artifact CAS | deterministic directory/file/chunk objects and reconstructible layer materializations | equal-content and successor deduplication tests |
| Existing lifecycle integration | CAS-backed sync/promotion/inheritance/GC using current generation and COW machinery | crash matrix plus native sibling isolation |
| Repository contract v2 | extensions to `trail.environment.toml`, including source export | parser/adversarial/custom-pipeline E2E tests |
| Adapter protocol v3 | incomplete discovery, resolution, validation, capability, and identity wire types | SDK golden/compatibility/malicious-frame tests |
| Trust evidence | capability profiles, secret taint, quarantine, deterministic attestation | sandbox, redaction, revocation, tamper tests |

## Goals / Non-Goals

**Goals:**

- Represent recognized but incomplete framework components honestly.
- Resolve dependency state explicitly without requiring every generated lock snapshot to enter Git.
- Reuse unchanged artifact bytes across lanes, successor builds, and different desired keys.
- Preserve existing immutable-lower/private-upper isolation and atomic generation activation.
- Make every reuse, rebuild, quarantine, rejection, export, and collection decision explainable.
- Let repository authors compose the same normalized pipeline through an extension of the existing environment file.
- Keep unproven framework output private and make generated-source publication explicit.
- Produce portable content identities and attestations without accepting remote content automatically.

**Non-Goals:**

- Replacing package managers or framework build schedulers.
- Sharing a live writable target, `node_modules`, `.next`, virtual environment, database, process, secret, or cache without a separately certified concurrent protocol.
- Inferring correctness inputs from filesystem observation or syscall tracing.
- Automatically resolving dependencies during managed execution unless workspace policy explicitly opts in for the adapter and authority set.
- Automatically writing resolution snapshots or generated output into source history.
- Adding remote cache transport, remote execution, registry hosting, or distributed leases.
- Migrating incompatible `.trail/` database shapes; Trail retains schema-v1 hard-cutover behavior.

## Decisions

### 1. Extend the shipped environment pipeline

The host operation remains desired-state convergence:

```text
discover pinned source facts (read-only)
  -> resolve missing dependency identity (optional and policy-controlled)
  -> finalize the existing typed component DAG
  -> compare desired keys with active generation/artifacts
  -> construct unresolved nodes through current bounded scheduling/singleflight
  -> validate and seal deterministic content
  -> activate one successor environment generation
  -> mount existing immutable lowers plus lane-private uppers
  -> execute and checkpoint source
  -> retire bindings and collect unreachable content
```

`resolve`, `construct`, `validate`, and `seal` have separate durable evidence, but they feed the current environment sync, promotion, generation, mount, retirement, and GC operations. There is no second artifact scheduler.

Alternatives rejected:

- A parallel artifact engine would duplicate generation and recovery authority.
- Framework-specific cache managers would repeat COW, identity, trust, and collection logic.
- A generic shell lifecycle would make argv, capabilities, inputs, outputs, and recovery ambiguous.

### 2. Discovery returns proposals, including incomplete components

Discovery is marker-driven and side-effect-free. A recognized manifest produces `ArtifactComponentProposalV1` even when a lock snapshot, tool, permission, or platform capability is missing.

| Status | Meaning |
| --- | --- |
| `ready` | All identity-bearing planning inputs are present and verified. |
| `resolvable` | An adapter supplied a resolver plan and current policy can authorize it. |
| `blocked` | The component is understood but a tool, authority, credential reference, platform feature, or approval is unavailable. |
| `unsupported` | The framework form is recognized but cannot be represented safely. |
| `ambiguous` | Multiple component roots/adapters claim one identity and explicit selection is required. |

The proposal contains stable reason codes and exact recovery commands. Discovery never invokes a package manager, compiler, repository action, network endpoint, runtime provider, or secret provider.

### 3. Resolution snapshots are environment metadata

`ArtifactResolutionPlanV1` pins:

- proposal and source-root identity;
- exact resolver executable and argv;
- readable manifest/config inputs;
- writable candidate path;
- allowed network authorities and opaque credential handles;
- script policy, environment-name roles, resource limits, and snapshot format;
- structural and ecosystem validation rules.

The resolver runs in a separate deny-by-default profile. On success Trail ingests a deterministic `ArtifactResolutionSnapshotV1` containing resolved identities, checksums, source proposal key, resolver/tool/policy identity, contacted authority names, predecessor, and format version. Credential bytes and authorization headers are never stored.

Snapshots remain pinned until their identity inputs change or the user requests `--refresh`. Time alone never advances dependency selection. Managed execution fails with the required `trail env resolve ...` command unless policy explicitly permits automatic resolution for the adapter and exact authority set.

A snapshot can be exported to a conventional lockfile only through the source-export operation in Decision 10.

### 4. Keep four independent identities

The domain model separates:

```text
ArtifactDesiredKeyV2
  what output is correct for a normalized plan and its declared inputs

ArtifactTreeRootV1
  exact deterministic directory/file/chunk content and relevant metadata

ArtifactEnvelopeV1 / ArtifactId
  desired key + tree root + output contract + portability/trust + attestation

ArtifactGenerationBinding
  lane view + generation + component/output + artifact or private storage
```

`ArtifactDesiredKeyV2` uses canonical CBOR and SHA-256 over:

- contract, adapter implementation, package distribution, and selected protocol;
- resolution snapshot identity;
- declared source/input closure and semantic normalizer versions;
- identity-bearing upstream desired/artifact identities;
- exact action executable identities, argv, working directories, and phase order;
- output, validation, source-export, network, script, secret-taint, sandbox, and publication policy;
- identity-affecting non-secret environment;
- target, platform, architecture, ABI, portability, reuse, and trust scope.

Maps and sets are sorted; paths are normalized NFC relative paths; absent and empty fields remain distinct; all enum and codec versions are explicit. When an adapter cannot prove a complete closure, the complete pinned Trail source root and conservative host dimensions enter the desired key, or reusable publication is refused.

The existing `WorkspaceLayerKeyV1` remains readable only for explicitly supported v1/v2 compatibility projections. New publications use v2 identity. A v1 key is never reinterpreted as v2.

### 5. Split pure content manifests from artifact provenance

The current layer manifest mixes tree hashes with layer/request/publication metadata. The new object graph is:

```text
ArtifactEnvelopeV1
  -> DirectoryNodeV1 root
       -> DirectoryNodeV1 children
       -> FileNodeV1
            -> BlobObjectV1                 (small file)
            -> ChunkListV1 -> ChunkObjectV1 (large file)
  -> ArtifactAttestationV1
```

Pure content nodes exclude desired keys, adapter names, timestamps, storage paths, layer IDs, generation IDs, and publication attempt IDs. They include normalized entry names, kind, executable/portable mode bits, safe symlink targets, file size, complete-file SHA-256, and child object identities. Equivalent trees produce the same root regardless of traversal order or producing component.

Artifact envelopes associate the pure root with the desired key, output contract, portability, trust scope, verification policy, and attestation. Different desired keys may point at the same tree root without sharing correctness identity.

### 6. Use versioned whole-file and content-defined chunk objects

Files at or below 1 MiB use one immutable blob object. Larger files use `fastcdc-v1` with minimum 256 KiB, target 1 MiB, and maximum 4 MiB chunks. Every file retains a full SHA-256 independent of chunking, and every chunk is independently SHA-256 addressed. Chunk profile and thresholds are versioned in `FileNodeV1`; changing them creates a new file-node identity without relabeling old objects.

Ingestion is streaming and bounded by declared entry, byte, depth, path, chunk, and time limits. It rejects absolute or traversing paths, non-NFC names, case collisions, escaping links, unsupported special files, prohibited metadata/xattrs, concurrent mutation, and secret-policy violations before publication.

Alternative rejected: per-layer full directory copies are simple but duplicate equal files and make layer directories the only authoritative bytes. Fixed-size chunks are simpler but amplify changes after insertions in large archives and package indexes.

### 7. CAS objects are authoritative; layer directories become materializations

Immutable content objects use Trail's existing object publication and validation boundary. SQLite stores durable references and coordination; the object graph stores authoritative content. A materialized layer under the current cache hierarchy is reconstructible and never the only copy of a ready artifact.

Backends consume content in two ways:

- lazy NFS/FUSE/Dokan projections resolve manifest entries and materialize requested file content on demand;
- backends requiring real directories use a verified materialization cache keyed by tree root and backend compatibility.

Implementation status: the shared backend-neutral workspace core resolves
verified artifact-tree bindings one directory object at a time, serves file
ranges from only the overlapping blob/chunks, and materializes only a selected
file for copy-up. It treats the CAS manifest as authoritative over any legacy
layer directory and preserves existing upper/whiteout behavior. Native FUSE,
NFS, and Dokan acceptance tests exercise the same nonexistent-materialization
fixture; owning-host gates remain the authority for platform qualification.
Backends that require real paths now reuse a verified real-directory cache keyed
by the authoritative tree root and an operating-system/architecture/backend
compatibility identity. Publication projects and verifies CAS content in an
attempt-owned stage, records durable `building`/`verified`/`failed` state, and
atomically publishes an immutable root. Reuse revalidates content and restores
read-only permissions; missing or corrupt cache bytes rebuild from CAS. Private
projections prefer native clone/reflink and fall back to file copies without
hard-linking mutable consumers to the authoritative cache. Cache eviction,
reachability accounting, and quotas remain tasks 6.6-6.8.

Materialization can use safe platform clone/reflink facilities or immutable content copying. Hard links are allowed only where mode and ownership invariants cannot mutate the content object. Materialization amplification is reported separately and is reclaimable without invalidating an artifact.

The current `.trail/cache/layers/<layer-id>` layout remains an implementation compatibility surface during rollout, but after CAS activation it contains only verified materialization state and sidecars. Doctor/fsck must detect legacy authoritative-layer layouts versus CAS-backed layouts exactly.

### 8. Reuse existing publication, promotion, and generation fences

The shipped publication/promotion operation already pins source/generation/output, quiesces private state, validates containment, publishes an immutable layer, and activates a successor generation. This change inserts content sealing into that path:

```text
reserved -> constructing -> candidate_closed -> validating
         -> sealing_objects -> envelope_ready -> activating -> activated
```

Terminal alternatives are `failed`, `cancelled`, `quarantined`, and `repair_required`. Objects become durable before an envelope becomes ready. The existing activation transaction then binds the ready artifact to a successor generation. A crash can leave an unbound ready artifact, never a partially active generation.

Promotion never mutates or deletes the live private upper. Source, desired key, generation, private-output journal, gate, or validation changes after the fence make the attempt stale and prevent activation.

### 9. Upgrade singleflight and quarantine nondeterminism

The existing key lock remains the fast mutual-exclusion boundary, but durable attempt rows record owner token/generation, PID/start identity, heartbeat, pins, current phase, candidate root, waiters, error, and recovery command. At most one live reusable constructor owns `(trust_scope, desired_key)`.

If two structurally valid constructions for one desired key and trust scope produce different tree roots, Trail:

1. marks the desired key quarantined;
2. retains bounded references to both candidates and their attestations;
3. prevents automatic shared attachment or publication for that key;
4. reports the differing roots, adapter/tool/policy identities, and available deterministic comparison;
5. optionally permits a policy-controlled lane-private rebuild/retention that is never reported as a shared hit.

Quarantine resolution never relabels bytes. A user can revoke a producer, narrow policy, replace the desired key contract, or clear a demonstrably corrupt candidate through an explicit audited operation.

### 10. Generated source uses explicit source export

`publishable_source` is not an artifact output policy. Artifact output stays under one of the four shipped storage policies. A separate `[[component.source_export]]` declaration identifies a validated candidate subtree and a repository-relative destination.

Export requires an explicit user operation unless organization policy authorizes a named deterministic gate. Trail revalidates the pinned desired key, artifact/tree identity, source root, destination containment, ignore and guardrail policy, collision behavior, secret policy, and current lane state. It then writes through the normal source-operation path so the result appears in lane diff/review and can be checkpointed and merged through Git.

Artifact promotion and source export are intentionally different:

| Operation | Destination | Git-visible | Automatic default |
| --- | --- | --- | --- |
| artifact promotion | immutable Trail artifact + generation | no | policy-controlled |
| source export | lane source upper | yes | never |

### 11. Extend `trail.environment.toml` with schema v2

Trail retains both supported source paths, `trail.environment.toml` and `.trail/environment.toml`. New behavior uses `schema = "trail.environment/v2"`. Version 1 keeps its exact current parsing and restrictions; v2 is opt-in and never changes v1 defaults silently.

Representative v2 composition:

```toml
schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"
missing_resolution = "explicit"

[[component]]
id = "web.dependencies"
root = "apps/web"
adapter = "trail/node@2"

[[component.input]]
path = "apps/web/package.json"
role = "identity"
format = "bytes"

[component.resolve]
command = ["pnpm", "install", "--lockfile-only", "--ignore-scripts"]
cwd = "apps/web"
network = { authorities = ["registry.npmjs.org"] }
snapshot = "pnpm-lock.yaml"
format = "application/vnd.pnpm.lock+yaml"

[[component.action]]
phase = "construct"
command = ["pnpm", "install", "--frozen-lockfile", "--offline", "--ignore-scripts"]
cwd = "apps/web"

[[component.output]]
name = "modules"
source = "node_modules"
target = "apps/web/node_modules"
policy = "immutable_seed_private"
reuse = "exact"
scope = "workspace"
publish = "on_sync"

[[component.validation]]
kind = "path_contract"
path = "node_modules"

[[component]]
id = "web.client"
root = "apps/web"
adapter = "trail/command@2"

[[component.edge]]
component = "web.dependencies"
type = "build_requires"

[[component.action]]
phase = "construct"
command = ["pnpm", "exec", "vite", "build"]
cwd = "apps/web"

[[component.output]]
name = "dist"
source = "dist"
target = ".trail-generated/web-dist"
policy = "immutable_shared"
publish = "successful_gate"
gate = "build"

[[component.source_export]]
from_output = "dist"
source = "generated-client"
target = "apps/web/src/generated-client"
mode = "explicit"
```

The v2 parser has strict unknown-field handling, bounded includes/profiles, fixed argv, normalized paths, deterministic pattern expansion, and capability narrowing. Repository-authored plans cannot request shells, raw secrets, host-wide reuse, provider sockets, arbitrary child processes, or compatible semantic reuse unless a certified profile supplies that authority.

### 12. Add adapter protocol v3 as a delta

`trail.environment-adapter/v3` adds only concepts absent from v2:

- proposal status, missing requirements, and recovery actions;
- resolution-plan and snapshot schemas;
- input roles and complete-closure certification;
- multiple typed action phases and validations;
- desired/content/envelope identity evidence;
- effective capability profile and secret-taint result;
- attestation and quarantine descriptors;
- explicit source-export declarations.

Negotiation selects the highest exact mutually supported version. V1/v2 responses go through their existing compatibility conversion and cannot request v3-only resolution, source export, attestation, or compatibility certification. Package digest and selected protocol participate in the desired key. SDK builders reject invalid combinations; the host repeats full validation at the trust boundary.

### 13. Separate resolver, constructor, validator, and execution trust

Capability profiles are deny-by-default and phase-specific:

| Profile | Network | Writes | Secrets | Publication authority |
| --- | --- | --- | --- | --- |
| discovery/planning | none | none | none | none |
| resolver | exact authorized authorities | isolated snapshot candidate | opaque handles only | none |
| constructor | offline by default | declared candidate/cache/temp | none by default | none |
| validator | none by default | receipt only | none | none |
| mounted execution | declared lane bindings | lane-private/source as authorized | runtime injection | none |

Reviewed built-ins, certified signed plugins, locally trusted plugins, and repository declarations have distinct maximum profiles. Host/workspace policy can narrow but never widen them. Unsupported native enforcement fails closed for untrusted actions.

Any producer receiving secret bytes is tainted and may create only private, non-promotable output in this version. Secret bytes never enter keys, snapshots, objects, manifests, logs, reports, shared caches, attestations, or future remote requests.

### 14. Attest sealed artifacts without polluting content identity

`ArtifactAttestationV1` deterministically records:

- desired key, tree root, artifact envelope, source root, resolution snapshot, and upstream identities;
- adapter implementation/package/publisher/protocol and revocation state at publication;
- executable identities, argv, platform/ABI, sandbox enforcement, network/script policy, and non-secret environment roles;
- output contract, validations, gates, attempt identity, and producer trust tier;
- portability and allowed sharing scope.

Wall-clock timestamps and local paths are stored as non-identity observation fields or separate attempt metadata. Artifact attachment verifies content, scope, producer trust/revocation, and required evidence. Portable identity does not imply remote trust; imported content remains untrusted until a future transport policy verifies signatures and local acceptance.

### 15. Extend existing public operations

Existing `env discover`, `graph`, `plan`, `explain`, `sync`, `promote`, `lane space`, and `cache gc` retain their grammar. Add:

```text
trail env resolve all [<lane>] [--refresh]
trail env resolve component <component> [--lane <lane>] [--refresh]
trail env artifact inspect <artifact-id>
trail env artifact verify <artifact-id> --level attach|sample|full|reproduce
trail env artifact quarantine list|show|resolve
trail env source export <lane> --component <id> --export <name>
```

All library, CLI, HTTP/OpenAPI, and MCP surfaces use shared report types. Reports include proposal state, desired/tree/artifact/binding identities, decision source, invalidating edges, attempt phase, verification/trust/quarantine state, logical/unique/shared/materialized/private bytes, and exact recovery commands.

### 16. Extend schema-v1 durable truth coherently

Fresh schema creation and validation add records for:

| Record | Durable purpose |
| --- | --- |
| resolution snapshots/attempts | source pins, snapshot object, resolver evidence, state, owner/recovery |
| artifact trees/envelopes | desired key, content root, manifest object, trust/verification/quarantine state |
| construction attempts/waiters | fenced ownership, phase, candidate, errors, bounded consumers |
| attestations | immutable statement object and optional signature metadata |
| quarantines/holds | divergent candidates, reason, retention, audited resolution |
| generation outputs | exact artifact envelope or private storage binding |
| content reachability index | rebuildable traversal/accounting acceleration, never sole truth |

Directory/file/chunk/snapshot/envelope/attestation objects are reachable from durable refs and generation bindings. GC traces object edges and retains active/retained generations, live/recoverable attempts, leases, quarantines, backups, and pins. Materialization and prefetch caches are independently reclaimable.

No database migration is added. An incompatible existing workspace fails before mutation with backup and `trail init --force` guidance. Backup/restore includes authoritative objects and retained private state, while explicitly reporting omitted reconstructible materializations and performance caches.

### 17. Frameworks are conformance compositions

| Shape | Resolution/content composition | Required conservative behavior |
| --- | --- | --- |
| Cargo | manifest proposal -> generated/source lock snapshot -> target seed/private upper | complete source-root fallback until workspace/features/build-script closure is certified |
| npm/pnpm/Yarn/Bun | package proposal -> frozen graph -> dependency seed/content cache | lifecycle-script writes isolated; unsupported lock form remains visible |
| Next.js | Node dependency component -> framework/config/build component | `.next/cache` private; output shareable only with relocatability validation |
| Vite | Node dependency component -> plugin/config/mode build | immutable `dist` or explicit source export; optimizer state private/seeded only when certified |
| Python | project proposal -> hash-bearing resolution -> wheels/downloads -> private `.venv` | path-bound environment and bytecode remain private |
| Go | module graph/checksum snapshot -> vendor/external module content | build cache uses certified cache protocol, not artifact authority |
| Maven/Gradle | dependency checksum graph -> immutable dependency objects | daemons and build directories remain private |
| CMake | toolchain/config inputs -> private configure/build tree | only certified compiler cache/content is shared |
| Bazel/Nix | imported verified external-store identities | avoid duplicating stores; workspace output remains private unless exported |
| custom | `trail.environment/v2` fixed graph/actions | unproven closure or portability narrows to private/exact workspace reuse |

Trail core contains storage and lifecycle concepts, not `nextjs`, `vite`, or package-manager-specific COW modes.

### 18. Verification and performance gates are evidence-based

The acceptance matrix must prove:

- discovery performs no process/network/provider side effects;
- equal desired keys singleflight to one verified artifact;
- different desired keys producing equal files reuse identical content objects;
- changing a bounded subset of a successor tree creates only affected directory/file/chunk objects;
- divergent content for one desired key quarantines all shared candidates;
- 1/5/20 sibling lanes share immutable content while every mutation/whiteout remains private;
- source export is the only path by which declared generated source enters a lane diff;
- retirement/GC preserves chunks reachable from any artifact and reclaims last-reference content deterministically;
- doctor, fsck, backup, and restore preserve or reconstruct the correct authority;
- 10k/100k/1M-entry artifacts remain bounded in memory, manifest paging, startup, and reporting;
- NFS, FUSE, and Dokan evidence is reported only from owning native hosts; skipped evidence remains unverified.

## Risks / Trade-offs

- **CAS object count and SQLite reachability can become large** -> page manifests, stream traversal, batch indexed lookups, incremental GC, and bound every operation.
- **Content-defined chunking adds CPU and dependency risk** -> version the profile, benchmark against whole-file storage, retain full-file hashes, and use whole blobs for small files.
- **Materialization caches can temporarily amplify disk use** -> report authoritative versus materialized bytes separately, enforce quotas, and reclaim materializations independently.
- **Resolution can introduce network drift** -> require pinned proposal inputs, explicit authority, immutable snapshots, manual refresh, and offline construction where supported.
- **Repository schema v2 can become a build language** -> keep fixed argv and finite typed phases, reject control flow/shell interpolation, and route semantic logic to adapters.
- **Source export can overwrite user work** -> require pinned source identity, explicit conflict policy, normal source guardrails, reviewable diff, and atomic confined writes.
- **Desired-key v2 invalidates existing cache hits** -> preserve read-only v1 inspection but never reinterpret identities; communicate hard-cutover and allow rebuild from source.
- **Native backend behavior differs** -> keep semantics in the host model and require backend-specific lower-integrity, whiteout, crash, and lazy-read evidence.

## Migration Plan

### Phase 0: Freeze the merged baseline

- Archive/synchronize `optimize-large-repo-lanes` separately when release workflow permits.
- Pin current v1/v2 wire fixtures, `trail.environment/v1`, output enums, promotion reports, schema shape, and native COW behavior.
- Add failing tests that demonstrate manifest-only discovery omission, desired/content conflation, and duplicate whole-directory bytes.

### Phase 1: Read-only proposals, identities, and planning

- Add proposal statuses, resolution plans, desired-key v2, tree/envelope models, and report projections without executing resolution or changing layer storage.
- Add `trail.environment/v2` parsing and graph normalization behind an explicit schema tag.
- Keep existing sync/publication behavior authoritative.

### Phase 2: Resolution snapshots

- Add resolver attempts, capability enforcement, snapshot objects, explicit CLI/API operations, recovery, and source materialization/export.
- Migrate Cargo and Node manifest-only discovery first.

### Phase 3: CAS shadow publication

- Build deterministic tree/file/chunk objects alongside current layer publication.
- Verify new content roots against current copied trees without attaching from CAS.
- Measure object count, chunking CPU, logical/physical bytes, and successor reuse.

### Phase 4: CAS-backed attachment and promotion

- Make new artifacts authoritative through envelope objects and materialization projections.
- Route existing sync, promotion, inheritance, verification, backup/restore, doctor/fsck, and GC through CAS-aware operations.
- Preserve old active generations only under the explicit hard-cutover/reinitialization contract.

### Phase 5: Quarantine, attestations, and protocol v3

- Activate nondeterminism detection, trust-scoped attestations, SDK v3, plugin negotiation, and malicious-input fixtures.
- Keep v1/v2 behavior exact and deny v3-only features to compatibility projections.

### Phase 6: Adapter and framework qualification

- Migrate a fixture adapter, Cargo, Node, Next.js/Vite composition, Python, and representative custom/external shapes.
- Run native multi-lane, scale, recovery, deduplication, and real-tool gates.

### Phase 7: Public hard cutover and release

- Update every public interface and reference document.
- Require backup/reinitialization guidance for incompatible schema-v1 state.
- Release only with exact passed, failed, and skipped platform evidence.

Rollback before CAS-backed activation disables new planning/resolution and leaves existing storage authoritative. After the schema-v1 hard cutover, rollback requires restoring a pre-cutover backup or reinitializing from Git; Trail does not attempt an in-place downgrade.

## Open Questions

None block implementation. The design fixes these choices: extend `trail.environment.toml`; keep four artifact output policies; model generated source as explicit source export; use desired-key v2 plus independent content roots; use SHA-256 and versioned FastCDC; preserve existing COW/promotion/generation machinery; negotiate protocol v3 explicitly; and defer remote transport.
