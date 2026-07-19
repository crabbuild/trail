# ACP Relay Final Outcome Design

## Goal

Make ACP turn finalization independent of which relay pump reports completion first.

## Design

The relay will continue draining and joining both pumps before capture finalization. After both results and the bounded child wait are available, a small pure classifier will derive the final capture reason with this precedence:

1. An agent-pump error is `AgentError`.
2. A non-timeout unsuccessful child exit is `AgentError`, even if editor EOF arrived first.
3. An editor-pump error is `EditorError`.
4. Otherwise preserve the first clean EOF reason. A child killed by the existing bounded-timeout path keeps the existing graceful-shutdown behavior.

The observer receives that classified reason before its barrier is flushed. Existing transport error propagation remains unchanged after capture has settled.

## Verification

Transport unit tests will prove that editor EOF cannot mask either an upstream nonzero exit or an agent parsing error. Focused ACP e2e tests will be repeated to cover scheduling variation, followed by formatting checks.
