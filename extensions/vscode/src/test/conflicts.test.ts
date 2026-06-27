import assert from "node:assert/strict";
import test from "node:test";
import { conflictSetIdsFromSources } from "../shared/conflicts";

test("conflictSetIdsFromSources extracts conflict ids from readiness blocker details", () => {
  const ids = conflictSetIdsFromSources({
    readiness: {
      blockers: [
        {
          code: "open_conflicts",
          message: "2 merge conflict sets are still open",
          details: { conflict_set_ids: ["conflict-a", "conflict-b"] }
        }
      ]
    }
  });

  assert.deepEqual(ids, ["conflict-a", "conflict-b"]);
});

test("conflictSetIdsFromSources extracts direct and nested conflict set ids", () => {
  const ids = conflictSetIdsFromSources(
    {
      conflicts: [{ conflict_set_id: "conflict-a" }, { conflictSetId: "conflict-b" }]
    },
    {
      review: {
        warnings: [
          {
            details: {
              conflicts: [{ conflict_set_id: "conflict-c" }],
              ignored: { conflictSetIds: ["conflict-d"] }
            }
          }
        ]
      }
    }
  );

  assert.deepEqual(ids, ["conflict-a", "conflict-b", "conflict-c", "conflict-d"]);
});

test("conflictSetIdsFromSources deduplicates and ignores prose-like values", () => {
  const ids = conflictSetIdsFromSources({
    blockers: [
      {
        details: {
          conflict_set_ids: ["conflict-a", "conflict-a", "conflict in src/app.ts"]
        }
      }
    ]
  });

  assert.deepEqual(ids, ["conflict-a"]);
});
