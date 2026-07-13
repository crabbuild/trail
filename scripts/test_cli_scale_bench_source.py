import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).with_name("cli-scale-bench.sh")


class CliScaleBenchSourceTests(unittest.TestCase):
    def test_unmapped_clone_uses_transport_instead_of_local_object_copy(self):
        source = SCRIPT.read_text()
        self.assertIn(
            'git clone --no-local --quiet "$GIT_REPO" "$GIT_UNMAPPED_REPO"',
            source,
        )


if __name__ == "__main__":
    unittest.main()
