import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).with_name("cli-scale-bench.sh")
ROOT = SCRIPT.parent.parent


class CliScaleBenchSourceTests(unittest.TestCase):
    def test_unmapped_clone_uses_transport_instead_of_local_object_copy(self):
        source = SCRIPT.read_text()
        self.assertIn(
            'git clone --no-local --quiet "$GIT_REPO" "$GIT_UNMAPPED_REPO"',
            source,
        )

    def test_git_plumbing_command_ceiling_is_constant_in_ci_and_scale_gates(self):
        for relative in [".github/workflows/ci.yml", ".github/workflows/scale.yml"]:
            with self.subTest(workflow=relative):
                source = (ROOT / relative).read_text()
                self.assertIn("agent_git_plumbing_commands=5", source)


if __name__ == "__main__":
    unittest.main()
