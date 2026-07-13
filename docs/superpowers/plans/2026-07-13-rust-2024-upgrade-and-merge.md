# Rust 2024 Upgrade and Merge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move Trail's parent-owned Rust crates to edition 2024 with an honest Rust 1.89 minimum, upgrade the installed stable toolchain, verify ACP v1 on that toolchain, and merge the feature branch into local `main`.

**Architecture:** The root workspace contract owns the edition and MSRV for `trail` and `trail-environment-adapter-sdk`; the independently versioned `prolly` git submodule keeps its own edition contract. Run Cargo's edition migration before changing the root manifest, update user-facing build documentation and the ACP release plan, then verify the feature branch and the merged main checkout separately.

**Tech Stack:** Rust edition 2024, Rust/Cargo 1.89 minimum, rustup stable, Cargo workspace manifests, Git linked worktrees.

## Global Constraints

- Do not modify or remove unrelated untracked files in `/Users/haipingfu/CrabDB`.
- Do not change the independently versioned `prolly` submodule's edition or commit pointer.
- Rust edition 2024 requires Rust 1.85; Trail's resolved dependencies raise the actual minimum to Rust 1.89.
- Merge only after the full Trail and ACP v1 verification commands pass on the upgraded toolchain.
- Remove the owned `.worktrees/acp-v1-conformance` worktree only after the merge and merged-result verification succeed.

---

### Task 1: Upgrade the Toolchain and Workspace Contract

**Files:**
- Modify: `Cargo.toml`
- Create: `rustfmt.toml`
- Modify: `Makefile`
- Modify: `README.md`
- Modify: `docs/getting-started/install-and-build.md`
- Modify: `docs/superpowers/plans/2026-07-12-acp-v1-full-conformance.md`

**Interfaces:**
- Consumes: the existing root `[workspace.package]` edition and `rust-version` inherited by Trail-owned crates.
- Produces: edition `2024`, `rust-version = "1.89"`, stable 2021-style formatting, and a `make toolchain` target that updates stable Rust.

- [ ] **Step 1: Upgrade the installed stable toolchain**

Run:

```sh
rustup update stable
rustup component add rustfmt clippy --toolchain stable
rustc +stable --version
cargo +stable --version
```

Expected: rustc and Cargo report version 1.89 or newer and both components install successfully.

- [ ] **Step 2: Run Cargo's edition migration before changing the manifest**

Run:

```sh
cargo +stable fix --edition -p trail -p trail-environment-adapter-sdk --all-targets --allow-dirty
```

Expected: Cargo applies any source changes required to preserve edition-2021 behavior under edition 2024.

- [ ] **Step 3: Update the workspace contract and toolchain helper**

Set the root workspace package values to:

```toml
edition = "2024"
rust-version = "1.89"
```

Change the Makefile toolchain recipe to run `rustup update stable` before installing `clippy` and `rustfmt` for stable.

Create `rustfmt.toml` with `style_edition = "2021"` so the language upgrade does not introduce a repository-wide cosmetic formatting rewrite.

- [ ] **Step 4: Update current build and ACP release documentation**

Replace current Trail build requirements and executable ACP MSRV checks from Rust 1.81/edition 2021 to Rust 1.89/edition 2024. Preserve the official ACP reference peer as an independent interoperability package.

- [ ] **Step 5: Verify the declared minimum**

Run:

```sh
cargo +1.89.0 check -p trail --all-targets
cargo +stable fmt --all -- --check
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 6: Commit the upgrade**

```sh
git add Cargo.toml Makefile README.md rustfmt.toml docs/getting-started/install-and-build.md docs/superpowers/plans/2026-07-12-acp-v1-full-conformance.md docs/superpowers/plans/2026-07-13-rust-2024-upgrade-and-merge.md trail trail-environment-adapter-sdk
git diff --cached --check
git commit -m "build: upgrade Trail to Rust 2024"
```

### Task 2: Verify Rust 2024 and ACP v1

**Files:**
- Modify only files required by failures caused by the edition migration.

**Interfaces:**
- Consumes: edition-2024 Trail crates and the completed ACP v1 implementation.
- Produces: fresh test, conformance, fault, interoperability, schema-drift, and benchmark evidence.

- [ ] **Step 1: Run the full Trail suite**

```sh
cargo +stable test -p trail --all-targets
```

Expected: zero failed tests.

- [ ] **Step 2: Run explicit ACP gates**

```sh
cargo +stable test -p trail --test acp_conformance --test acp_faults -- --nocapture
scripts/test-acp-v1-reference-interop.sh
scripts/check-acp-v1-schema-drift.sh
cargo +stable bench -p trail --bench acp_relay_bench
```

Expected: all 23 methods, stable variants, 266 capability shapes, fault cases, both interoperability peers, schema drift, and the 10,000-frame benchmark pass.

- [ ] **Step 3: Verify the branch is clean**

```sh
cargo +stable fmt --all -- --check
git diff --check
git status --short
```

Expected: no output from the two Git commands.

### Task 3: Merge and Verify Main

**Files:**
- No planned source edits.

**Interfaces:**
- Consumes: verified `feat/acp-v1-conformance` and local `main`.
- Produces: local `main` containing the ACP v1 and Rust 2024 commits.

- [ ] **Step 1: Update and merge main without disturbing untracked paths**

From `/Users/haipingfu/CrabDB`, verify the untracked paths do not collide with incoming tracked paths, update `main` with `git pull --ff-only`, and run:

```sh
git merge --no-ff feat/acp-v1-conformance
```

Expected: merge succeeds without altering the pre-existing untracked paths.

- [ ] **Step 2: Verify the merged result**

```sh
cargo +stable check -p trail --all-targets
cargo +stable test -p trail --all-targets
cargo +stable fmt --all -- --check
git diff --check
```

Expected: every command exits zero.

- [ ] **Step 3: Clean up the owned worktree and feature branch**

From `/Users/haipingfu/CrabDB`:

```sh
git worktree remove /Users/haipingfu/CrabDB/.worktrees/acp-v1-conformance
git worktree prune
git branch -d feat/acp-v1-conformance
```

Expected: `main` remains checked out, the worktree registration is removed, and the merged feature branch is deleted.
