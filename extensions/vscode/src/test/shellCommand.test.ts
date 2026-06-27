import assert from "node:assert/strict";
import test from "node:test";
import { shellCommandForPlatform } from "../crabdb/ShellCommand";

test("shellCommandForPlatform preserves POSIX command lines", () => {
  assert.deepEqual(shellCommandForPlatform("npm test -- --runInBand", "darwin"), [
    "sh",
    "-lc",
    "npm test -- --runInBand"
  ]);
  assert.deepEqual(shellCommandForPlatform("cat package.json | jq '.name'", "linux"), [
    "sh",
    "-lc",
    "cat package.json | jq '.name'"
  ]);
});

test("shellCommandForPlatform wraps Windows command lines", () => {
  assert.deepEqual(shellCommandForPlatform("npm test", "win32"), ["cmd.exe", "/d", "/s", "/c", "npm test"]);
});

test("shellCommandForPlatform rejects empty commands", () => {
  assert.throws(() => shellCommandForPlatform("   "), /Command cannot be empty/);
});
