# Fuse COW Hard-Cutover Implementation Plan

> **For agentic workers:** Execute inline. The user explicitly waived test-first TDD;
> update tests with each implementation slice and run them as verification gates.

**Goal:** Remove `overlay-cow` completely and expose distinct `fuse-cow` and
`dokan-cow` workdir modes across Trail.

**Architecture:** `LaneWorkdirMode` names the concrete transport. FUSE, NFS, and Dokan
share `ViewCore` semantics but have separate dispatch and user-visible identities. No
old spelling, parser alias, stored-state adoption, or migration remains.

**Tech Stack:** Rust, Clap, Serde, SQLite lane metadata, FUSE/fuser, Dokan, shell and
PowerShell verification scripts, Markdown documentation.

## Global constraints

- Preserve `full-cow` as a first-class mode.
- Do not accept `overlay-cow`, `overlay_cow`, or `OverlayCow` anywhere.
- Do not modify unrelated untracked projects or the parent checkout.
- Keep platform dispatch explicit: FUSE on Linux/macOS, Dokan on Windows, NFS on macOS.
- Run formatting, focused tests, the complete Trail suite, and cross-target checks.

---

### Task 1: Workdir-mode model and public contracts

**Files:**

- Modify: `trail/src/model/reports/lane.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/cli/command/lane_args.rs`
- Modify: `trail/src/cli/command/agent_args.rs`
- Modify: `trail/src/server/openapi/schemas/lane.rs`
- Modify: `trail/src/mcp/tools/lane.rs`
- Modify: affected parser/report tests in `trail/src/cli/command.rs` and
  `trail/tests/e2e.rs`

Steps:

- [ ] Add `FuseCow` and `DokanCow`; remove `OverlayCow`.
- [ ] Serialize/parse only the new names and return exact backend identifiers.
- [ ] Make automatic Windows selection `DokanCow` and Linux FUSE selection `FuseCow`.
- [ ] Validate platform availability before lane creation.
- [ ] Update CLI/OpenAPI/MCP enums and parser/report assertions.
- [ ] Run model, CLI, and lane-spawn focused tests.

### Task 2: Separate FUSE and Dokan backend dispatch

**Files:**

- Rename: `trail/src/db/lane/workdir/overlay.rs` to
  `trail/src/db/lane/workdir/fuse.rs`
- Rename: `trail/src/db/lane/workdir/overlay/dokan_overlay.rs` to
  `trail/src/db/lane/workdir/dokan.rs`
- Modify: `trail/src/db/lane/workdir.rs`
- Modify: lane lifecycle, view, gate, record, merge, agent, environment, and adapter
  call sites under `trail/src/db` and `trail/src/cli`

Steps:

- [ ] Rename FUSE types/functions to `FuseCow*`/`fuse_cow_*`.
- [ ] Rename Dokan types/functions to `DokanCow*`/`dokan_cow_*`.
- [ ] Add backend-specific Trail mount/prepare/candidate helpers.
- [ ] Dispatch `FuseCow`, `NfsCow`, and `DokanCow` explicitly at every mount site.
- [ ] Rename mount FS names, subtypes, diagnostics, and runtime state directories.
- [ ] Run library tests and Windows cross-checks for backend dispatch.

### Task 3: Metadata hard cutover and lifecycle behavior

**Files:**

- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/db/lane/workdir/lifecycle.rs`
- Modify: `trail/src/db/core/doctor_storage.rs`
- Test: `trail/tests/e2e.rs`

Steps:

- [ ] Persist only `fuse-cow`/`dokan-cow` and exact `cow_backend` values.
- [ ] Make old stored metadata fail with the recreate-lane diagnostic.
- [ ] Clean only new backend state paths; never adopt `.trail/overlay-cow`.
- [ ] Update doctor/backend availability checks.
- [ ] Add hard-cutover metadata and lifecycle assertions.

### Task 4: Scripts, workflows, docs, and skills

**Files:**

- Rename FUSE scripts containing `overlay-cow` to `fuse-cow`.
- Modify: `.github/workflows/layered-workspaces.yml`
- Modify: remaining scripts and PowerShell fixtures.
- Modify: current docs under `docs/`, `plans/`, and `skills/use-trail/`.

Steps:

- [ ] Rename flags, variables, volumes, files, headings, examples, and diagnostics.
- [ ] Use `dokan-cow` in Windows fixtures and `fuse-cow` in FUSE fixtures.
- [ ] Preserve generic “overlay semantics” only where it describes the algorithm.
- [ ] Verify checked-in current sources contain no removed product spelling or symbol.

### Task 5: Verification and regression repair

Steps:

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run focused parser, CLI, lane, environment, FUSE, NFS, and Dokan tests.
- [ ] Run `cargo test -p trail` and repair every failure caused by or exposed during the
  cutover.
- [ ] Run `cargo check -p trail --target x86_64-pc-windows-msvc` when the target is
  installed; otherwise record the missing external gate.
- [ ] Run repository absence scans excluding `.git`, `target`, and the historical design
  spec/implementation plan whose purpose is to describe the removed name.
- [ ] Inspect the final diff for unrelated changes and request code review.
