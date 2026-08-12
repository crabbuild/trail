import importlib.util
import os
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("edit-real-framework-semantic.py")
SPEC = importlib.util.spec_from_file_location("real_framework_editor", SCRIPT)
assert SPEC and SPEC.loader
EDITOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EDITOR)


BASE_FILES = {
    "go": {
        "version/version.go": "package version\n",
        "version/version_test.go": "",
    },
    "go-workspace": {
        "common/must.go": "package common\n",
        "common/must_test.go": (
            "package common_test\n\n"
            'import (\n\t"testing"\n\t"github.com/stretchr/testify/assert"\n'
            '\t"github.com/oxia-db/oxia/common"\n)\n'
        ),
    },
    "yarn": {
        "index.js": "module.exports = function isOdd() {}\n",
        "test.js": 'var assert = require("assert");\n',
    },
    "bun": {
        "src/app.ts": "export const app = {}\n",
        "tests/app.test.ts": 'import { test } from "bun:test";\n',
    },
    "uv": {
        "src/pyprojectx/__init__.py": "",
        "tests/unit/test_trail_qualification.py": "",
    },
    "pnpm": {
        "src/constants.ts": "export const value = 1;\n",
        "tests/http-helpers/index.test.ts": "import { describe, expect, it } from 'vitest';\n",
    },
    "npm": {
        "src/version.ts": "export default function version() {}\n",
        "src/test/version.test.ts": "import assert from 'node:assert';\n",
    },
    "python": {
        "src/tap/line.py": "class Line:\n    pass\n",
        "tests/test_line.py": "import unittest\n",
    },
    "cmake": {
        "util/hash.cc": "namespace leveldb {}\n",
        "util/hash.h": "#pragma once\n",
    },
    "cmake-modern": {
        "examples/minimal.cpp": "#include <CLI/CLI.hpp>\nint main() { return 0; }\n",
    },
}


class RealFrameworkSemanticEditorTests(unittest.TestCase):
    def fixture(self, root: pathlib.Path, framework: str) -> None:
        for relative, contents in BASE_FILES[framework].items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")

    def in_root(self, root: pathlib.Path):
        class WorkingDirectory:
            def __enter__(self_inner):
                self_inner.previous = pathlib.Path.cwd()
                os.chdir(root)

            def __exit__(self_inner, *_):
                os.chdir(self_inner.previous)

        return WorkingDirectory()

    def test_applies_cumulative_lane_markers_without_touching_other_paths(self):
        for framework in BASE_FILES:
            with self.subTest(framework=framework), tempfile.TemporaryDirectory() as temp:
                root = pathlib.Path(temp)
                self.fixture(root, framework)
                before = sorted(path.relative_to(root) for path in root.rglob("*") if path.is_file())
                with self.in_root(root):
                    for path, _ in EDITOR.contract(framework, "agent-a"):
                        EDITOR.verify_block(path, None)
                    previous = None
                    for lane in ("agent-a", "agent-b", "agent-c"):
                        for path, replacement in EDITOR.contract(framework, lane):
                            EDITOR.replace_block(path, previous, replacement)
                            EDITOR.verify_block(path, lane)
                        previous = lane
                after = sorted(path.relative_to(root) for path in root.rglob("*") if path.is_file())
                self.assertEqual(after, before)

    def test_rejects_skipping_a_parent_marker(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            self.fixture(root, "go")
            with self.in_root(root):
                with self.assertRaisesRegex(AssertionError, "no complete qualification block"):
                    for path, replacement in EDITOR.contract("go", "agent-b"):
                        EDITOR.replace_block(path, "agent-a", replacement)

    def test_rejects_duplicate_or_out_of_block_markers(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            self.fixture(root, "python")
            with self.in_root(root):
                path, replacement = EDITOR.contract("python", "agent-a")[0]
                path.write_text(
                    f'# agent-a outside\n{replacement}{replacement}', encoding="utf-8"
                )
                with self.assertRaisesRegex(AssertionError, "exactly one"):
                    EDITOR.verify_block(path, "agent-a")

    def test_yarn_and_bun_policy_invalidation_is_exact_and_nonreentrant(self):
        for framework, policy_path in (("yarn", ".yarnrc"), ("bun", "bunfig.toml")):
            with self.subTest(framework=framework), tempfile.TemporaryDirectory() as temp:
                root = pathlib.Path(temp)
                self.fixture(root, framework)
                if framework == "bun":
                    (root / policy_path).write_text(
                        '[test]\npreload = ["./tests/setup.ts"]\n', encoding="utf-8"
                    )
                with self.in_root(root):
                    path, replacement = EDITOR.invalidation_contract(framework)
                    EDITOR.replace_block(path, None, replacement)
                    self.assertEqual(path.read_text(encoding="utf-8").count(EDITOR.START), 1)
                    with self.assertRaisesRegex(
                        AssertionError, "already contains a qualification block"
                    ):
                        EDITOR.replace_block(path, None, replacement)


if __name__ == "__main__":
    unittest.main()
