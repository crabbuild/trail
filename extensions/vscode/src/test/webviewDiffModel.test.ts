import assert from "node:assert/strict";
import test from "node:test";
import { buildDiffModel, roughDiffStats } from "../webview/diffModel";

test("builds line and word-level diff evidence", () => {
  const model = buildDiffModel("src/app.ts", "const label = 'old';\nkeep\n", "const label = 'new';\nkeep\n");

  assert.equal(model.kind, "modified");
  assert.equal(model.additions, 1);
  assert.equal(model.deletions, 1);
  assert.match(model.rawDiff, /src\/app\.ts \(before\)/);

  const changed = model.rows.find((row) => row.kind === "modified");
  assert.ok(changed, "expected one modified row");
  assert.ok(changed.oldSegments?.some((segment) => segment.tone === "removed" && segment.text.includes("old")));
  assert.ok(changed.newSegments?.some((segment) => segment.tone === "added" && segment.text.includes("new")));
});

test("compacts unchanged context around distant changes", () => {
  const oldLines = Array.from({ length: 120 }, (_, index) => `line ${index + 1}`);
  const newLines = oldLines.slice();
  newLines[60] = "line 61 changed";

  const model = buildDiffModel("notes.txt", oldLines.join("\n"), newLines.join("\n"));

  assert.equal(model.tooLarge, false);
  assert.ok(model.rows.some((row) => row.kind === "gap"), "expected hidden unchanged context");
  assert.ok(model.omittedRows > 0);
});

test("uses a bounded fallback for oversized diffs", () => {
  const oldText = `${"old\n".repeat(45_000)}`;
  const newText = `${"new\n".repeat(45_000)}`;

  const model = buildDiffModel("large.log", oldText, newText);

  assert.equal(model.tooLarge, true);
  assert.equal(model.rows.length, 40);
  assert.ok(model.rawDiff.includes("@@ large diff preview truncated @@"));
});

test("rough stats avoid loading the full diff parser", () => {
  const stats = roughDiffStats("", "hello\nworld");

  assert.equal(stats.kind, "created");
  assert.equal(stats.oldLineCount, 0);
  assert.equal(stats.newLineCount, 2);
});
