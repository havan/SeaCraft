"""Tests for the pure parts of build.py: python3 -m unittest site/test_build.py

They cover how build.py reads seacraft's output, never a number of its own: the numbers come from the
program, and its own tests check them.
"""

import importlib.util
import re
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("build", HERE / "build.py")
build = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(build)


class TestReleaseVersion(unittest.TestCase):
    def test_reads_workspace_version(self):
        with tempfile.TemporaryDirectory() as scratch:
            cargo = Path(scratch) / "Cargo.toml"
            cargo.write_text('[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.6.0"\n', encoding="utf-8")
            self.assertEqual(build.release_version(cargo), "0.6.0")

    def test_repo_version_is_plain(self):
        self.assertRegex(build.release_version(build.ROOT / "Cargo.toml"), r"^\d+\.\d+\.\d+$")


if __name__ == "__main__":
    unittest.main()
