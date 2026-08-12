#!/usr/bin/env python3
"""Apply or verify one deterministic semantic edit in a qualification lane."""

from __future__ import annotations

import argparse
from pathlib import Path


START = "TRAIL REAL FRAMEWORK QUALIFICATION START"
END = "TRAIL REAL FRAMEWORK QUALIFICATION END"
LANES = {"agent-a", "agent-b", "agent-c"}


def block(comment: str, body: str) -> str:
    return f"{comment} {START}\n{body.rstrip()}\n{comment} {END}\n"


def replace_block(path: Path, expected: str | None, replacement: str) -> None:
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    if text.count(START) > 1 or text.count(END) > 1:
        raise AssertionError(f"{path} contains multiple qualification blocks")
    start = text.find(START)
    end = text.find(END)
    if expected is None:
        if start != -1 or end != -1:
            raise AssertionError(f"{path} already contains a qualification block")
        separator = "" if not text or text.endswith("\n\n") else "\n"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text + separator + replacement, encoding="utf-8")
        return
    if start == -1 or end == -1 or end < start:
        raise AssertionError(f"{path} has no complete qualification block")
    line_start = text.rfind("\n", 0, start) + 1
    line_end = text.find("\n", end)
    line_end = len(text) if line_end == -1 else line_end + 1
    current = text[line_start:line_end]
    if expected not in current:
        raise AssertionError(
            f"{path} qualification block does not contain expected marker {expected!r}"
        )
    path.write_text(text[:line_start] + replacement + text[line_end:], encoding="utf-8")


def verify_block(path: Path, expected: str | None) -> None:
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    start_count = text.count(START)
    end_count = text.count(END)
    if expected is None:
        if start_count or end_count:
            raise AssertionError(f"{path} unexpectedly contains a qualification block")
        return
    if start_count != 1 or end_count != 1:
        raise AssertionError(f"{path} does not contain exactly one qualification block")
    start = text.find(START)
    end = text.find(END)
    if end < start:
        raise AssertionError(f"{path} qualification block is malformed")
    line_start = text.rfind("\n", 0, start) + 1
    line_end = text.find("\n", end)
    line_end = len(text) if line_end == -1 else line_end + 1
    if expected not in text[line_start:line_end]:
        raise AssertionError(f"{path} does not contain qualification marker {expected!r}")


def contract(framework: str, marker: str) -> list[tuple[Path, str]]:
    if framework == "go":
        return [
            (
                Path("version/version.go"),
                block("//", f'const TrailQualificationMarker = "{marker}"'),
            ),
            (
                Path("version/version_test.go"),
                block(
                    "//",
                    "package version\n\n"
                    'import "testing"\n\n'
                    "func TestTrailQualificationMarker(t *testing.T) {\n"
                    f'\tif TrailQualificationMarker != "{marker}" {{\n'
                    f'\t\tt.Fatalf("stale qualification marker: got %q, want {marker}", TrailQualificationMarker)\n'
                    "\t}\n"
                    "}",
                ),
            ),
        ]
    if framework == "pnpm":
        return [
            (
                Path("src/constants.ts"),
                block("//", f'export const trailQualificationMarker = "{marker}";'),
            ),
            (
                Path("tests/http-helpers/index.test.ts"),
                block(
                    "//",
                    'describe("Trail qualification marker", () => {\n'
                    '  it("executes the current lane source", async () => {\n'
                    '    const { trailQualificationMarker } = await import("../../src/constants");\n'
                    f'    expect(trailQualificationMarker).toBe("{marker}");\n'
                    "  });\n"
                    "});",
                ),
            ),
        ]
    if framework == "npm":
        return [
            (
                Path("src/version.ts"),
                block("//", f"export const trailQualificationMarker = '{marker}';"),
            ),
            (
                Path("src/test/version.test.ts"),
                block(
                    "//",
                    "import { trailQualificationMarker } from '../version.js';\n\n"
                    "describe('Trail qualification marker', () => {\n"
                    "  test('executes the current lane build', () => {\n"
                    f"    assert.strictEqual(trailQualificationMarker, '{marker}');\n"
                    "  });\n"
                    "});",
                ),
            ),
        ]
    if framework == "python":
        return [
            (
                Path("src/tap/line.py"),
                block("#", f'TRAIL_QUALIFICATION_MARKER = "{marker}"'),
            ),
            (
                Path("tests/test_line.py"),
                block(
                    "#",
                    "class TestTrailQualificationMarker(unittest.TestCase):\n"
                    "    def test_trail_qualification_marker(self):\n"
                    "        from tap.line import TRAIL_QUALIFICATION_MARKER\n\n"
                    f'        self.assertEqual(TRAIL_QUALIFICATION_MARKER, "{marker}")',
                ),
            ),
        ]
    if framework == "cmake":
        return [
            (
                Path("util/hash.h"),
                block(
                    "//",
                    "namespace leveldb {\n"
                    "const char* TrailQualificationMarker();\n"
                    "}  // namespace leveldb\n"
                    f"// marker: {marker}",
                ),
            ),
            (
                Path("util/hash.cc"),
                block(
                    "//",
                    "namespace leveldb {\n"
                    "const char* TrailQualificationMarker() {\n"
                    f'  return "{marker}";\n'
                    "}\n"
                    "}  // namespace leveldb",
                ),
            ),
        ]
    raise AssertionError(f"unsupported framework {framework!r}")


def expected_previous(marker: str) -> str | None:
    return {
        "baseline": None,
        "agent-a": "agent-a",
        "agent-b": "agent-b",
        "agent-c": "agent-c",
    }[marker]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("edit", "verify"))
    parser.add_argument("framework", choices=("go", "pnpm", "npm", "python", "cmake"))
    parser.add_argument("marker", choices=("baseline", *sorted(LANES)))
    args = parser.parse_args()

    if args.action == "edit":
        if args.marker not in LANES:
            raise AssertionError("baseline cannot be applied as an edit")
        previous = {"agent-a": None, "agent-b": "agent-a", "agent-c": "agent-b"}[
            args.marker
        ]
        for path, replacement in contract(args.framework, args.marker):
            replace_block(path, previous, replacement)
    else:
        expected = expected_previous(args.marker)
        for path, _ in contract(args.framework, args.marker if expected else "agent-a"):
            verify_block(path, expected)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
