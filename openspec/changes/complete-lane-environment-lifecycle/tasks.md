## 1. Durable retirement model

- [x] 1.1 Add schema v21 `lane_retirements`, exact schema validation, v20-to-v21 migration, backup/restore, and downgrade-refusal tests
- [x] 1.2 Add serialized retirement phase, operation, provenance, and report models plus read APIs
- [x] 1.3 Add failing lifecycle tests for archive/unarchive, completed removal cleanup, shared-layer preservation, and same-name reuse
- [x] 1.4 Implement archive and unarchive across database, CLI, HTTP, MCP, renderers, and docs
- [x] 1.5 Implement removal preparation with confined private-path manifests and compact provenance
- [x] 1.6 Implement idempotent runtime/mount/observer shutdown and generation/layer unbinding
- [x] 1.7 Implement private-upper/workdir deletion and completed tombstone compaction
- [x] 1.8 Add crash injection at every phase and automatic/retry recovery tests
- [x] 1.9 Implement forced purge with exact-identity safety and provenance deletion tests
- [x] 1.10 Verify cache GC reclaims unique layers and preserves shared layers after removal

## 2. Fork-time environment inheritance

- [x] 2.1 Add failing tests proving child forks share compatible layer IDs but have distinct source/generated/scratch uppers
- [x] 2.2 Add compatibility evaluation for adapter identity, component key, output policy, portability scope, source root, and layer verification
- [x] 2.3 Create child generation snapshots and copy compatible component/output/cache provenance without runtime, secret, lease, or sync state
- [x] 2.4 Attach inherited immutable layers during lane-spawn association and report inherited/rejected component reasons
- [x] 2.5 Add mixed-compatible, absent-generation, corrupted-layer, concurrent-fork, and parent-removal tests

## 3. Managed execution lifecycle

- [x] 3.1 Add failing orchestration tests for ordered prepare/finalize phases and aggregate failures
- [x] 3.2 Add the managed execution module with discover/plan, sync-all, runtime reconcile, mount, command launch, checkpoint, disposal, and unmount receipts
- [x] 3.3 Route lane exec, lane test, and lane eval through managed execution
- [x] 3.4 Route terminal-agent launch and finalization through managed execution
- [x] 3.5 Route ACP execution and finalization through managed execution
- [x] 3.6 Ensure test/eval/agent checkpoints include source changes and exclude generated/scratch artifacts
- [x] 3.7 Add interruption, preparation failure, command failure, checkpoint failure, cleanup failure, and crash-recovery integration tests

## 4. Verification and documentation

- [x] 4.1 Update CLI/API/MCP references and environment adapter contract for lifecycle semantics
- [x] 4.2 Run schema, lane initialization, workspace layer/runtime, agent/ACP, library, and E2E suites
- [x] 4.3 Run macOS NFS-COW Node, Cargo, Next.js/Vite, runtime cleanup, fork reuse, and remove/GC acceptance probes
- [x] 4.4 Audit every OpenSpec scenario against direct test or runtime evidence and resolve all gaps
