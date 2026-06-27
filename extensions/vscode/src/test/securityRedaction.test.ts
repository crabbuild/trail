import assert from "node:assert/strict";
import test from "node:test";
import {
  redactCommandArgs,
  redactString,
  redactValue,
  redactedJson
} from "../shared/securityRedaction";

test("redacts sensitive object keys recursively", () => {
  const redacted = redactValue({
    token: "abc123",
    nested: {
      api_key: "secret-value",
      safe: "visible"
    }
  }) as Record<string, unknown>;

  assert.equal(redacted.token, "[REDACTED]");
  assert.deepEqual(redacted.nested, {
    api_key: "[REDACTED]",
    safe: "visible"
  });
});

test("redacts sensitive command flag values", () => {
  assert.deepEqual(redactCommandArgs(["--token", "abc123", "--name", "task"]), [
    "--token",
    "[REDACTED]",
    "--name",
    "task"
  ]);
  assert.deepEqual(redactCommandArgs(["--api-key=abc123", "--provider", "claude"]), [
    "--api-key=[REDACTED]",
    "--provider",
    "claude"
  ]);
});

test("redacts common secret string forms", () => {
  assert.equal(redactString("Authorization: Bearer abc123"), "Authorization: Bearer [REDACTED]");
  assert.equal(redactString("password=hunter2 token=abc123"), "password=[REDACTED] token=[REDACTED]");
});

test("handles circular values in redacted json", () => {
  const value: Record<string, unknown> = {
    safe: "visible"
  };
  value.self = value;

  const json = redactedJson(value);
  assert.match(json, /"safe": "visible"/);
  assert.match(json, /"self": "\[Circular\]"/);
});
