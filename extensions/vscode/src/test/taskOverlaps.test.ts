import assert from "node:assert/strict";
import test from "node:test";
import { findTaskOverlaps } from "../shared/taskOverlaps";

test("findTaskOverlaps reports shared changed paths with active tasks", () => {
  const overlaps = findTaskOverlaps(
    {
      id: "task-a",
      lane: "lane-a",
      title: "Current",
      status: "active",
      changedPaths: ["src/app.ts", "README.md"]
    },
    [
      {
        id: "task-b",
        lane: "lane-b",
        title: "Other",
        status: "dirty",
        provider: "claude-code",
        changedPaths: ["src/app.ts", "src/other.ts"]
      }
    ]
  );

  assert.equal(overlaps.length, 1);
  assert.equal(overlaps[0]?.lane, "lane-b");
  assert.equal(overlaps[0]?.provider, "claude-code");
  assert.deepEqual(overlaps[0]?.sharedPaths, ["src/app.ts"]);
});

test("findTaskOverlaps ignores current and inactive tasks", () => {
  const overlaps = findTaskOverlaps(
    {
      id: "task-a",
      lane: "lane-a",
      title: "Current",
      status: "active",
      changedPaths: ["src/app.ts"]
    },
    [
      {
        id: "task-a",
        lane: "lane-copy",
        title: "Same id",
        status: "active",
        changedPaths: ["src/app.ts"]
      },
      {
        id: "task-c",
        lane: "lane-c",
        title: "Applied",
        status: "applied",
        changedPaths: ["src/app.ts"]
      }
    ]
  );

  assert.deepEqual(overlaps, []);
});

test("findTaskOverlaps normalizes slash style and deduplicates shared paths", () => {
  const overlaps = findTaskOverlaps(
    {
      id: "task-a",
      lane: "lane-a",
      title: "Current",
      status: "active",
      changedPaths: ["./src/app.ts"]
    },
    [
      {
        id: "task-b",
        lane: "lane-b",
        title: "Other",
        status: "blocked",
        changedPaths: ["src\\app.ts", "src\\app.ts"]
      }
    ]
  );

  assert.deepEqual(overlaps[0]?.sharedPaths, ["src\\app.ts"]);
});
