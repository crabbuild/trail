# ACP Relay Final Outcome Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finalize ACP capture from both pump results and the child exit instead of the first pump alone.

**Architecture:** Add a pure outcome classifier beside the transport loop. The loop drains both pumps and waits for the child as it does today, then calls the classifier before observer flush and existing error propagation.

**Tech Stack:** Rust standard library, Trail ACP transport, Cargo unit and integration tests.

## Global Constraints

- Do not modify `trail/tests/e2e.rs`.
- Preserve the existing bounded-timeout graceful shutdown behavior.
- Preserve existing transport error return behavior after capture finalization.

---

### Task 1: Classify the final ACP capture outcome

**Files:**
- Modify: `trail/src/acp/transport.rs:110-168`
- Test: `trail/src/acp/transport.rs:392-456`

**Interfaces:**
- Consumes: the first clean EOF reason, editor and agent `io::Result<()>` values, and an optional unsuccessful child-exit message.
- Produces: `final_finish_reason(...) -> RelayFinishReason`.

- [ ] **Step 1: Write the failing transport unit tests**

Add tests that expect `AgentError` when editor EOF is first but either the child exits unsuccessfully or the agent pump reports malformed input.

- [ ] **Step 2: Run the focused unit test and verify RED**

Run: `cargo test -p trail --lib final_finish_reason -- --nocapture`

Expected: compilation or assertion failure because the classifier does not exist.

- [ ] **Step 3: Implement the minimal classifier and call it after both pumps and child exit**

Give agent-pump and non-timeout child failures precedence over editor failures, then preserve the first clean EOF reason.

- [ ] **Step 4: Run focused and integration verification**

Run:

```text
cargo test -p trail --lib final_finish_reason -- --nocapture
cargo test -p trail --test e2e acp_relay_closes_failed_turn_on_upstream_crash -- --exact
cargo test -p trail --test e2e acp_relay_closes_failed_turn_on_malformed_upstream_json -- --exact
cargo fmt --all -- --check
```

Expected: all commands pass.

- [ ] **Step 5: Commit the ACP transport fix**

Commit `trail/src/acp/transport.rs` with a focused bug-fix message.
