#!/usr/bin/env python3
"""Offline control-flow regression: stale V2 directories must not select a prover."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class DispatchTest(unittest.TestCase):
    def run_checker(self, manifest, v2_checkout=True):
        with tempfile.TemporaryDirectory(prefix="snip36 dispatch ") as directory:
            root = Path(directory)
            scripts = root / "scripts"
            scripts.mkdir()
            shutil.copyfile(ROOT / "scripts/check-low-memory-prover.sh", scripts / "check.sh")
            v2 = scripts / "check-low-memory-prover-v2.sh"
            v2.write_text('#!/bin/sh\nprintf "V2-checker\\n"\n')
            v2.chmod(0o755)
            deps = root / "deps"
            (deps / "sequencer").mkdir(parents=True)
            (deps / "sequencer/Cargo.toml").write_text(manifest)
            if v2_checkout:
                (deps / "low-memory-prover/proving-v2").mkdir(parents=True)
            fixture = root / "vendor/low-memory-prover/check"
            fixture.mkdir(parents=True)
            (fixture / "Cargo.toml").write_text("# mock V1 fixture\n")
            binaries = root / "bin"
            binaries.mkdir()
            cargo = binaries / "cargo"
            cargo.write_text('#!/bin/sh\nprintf "V1-checker\\n"\nexit 73\n')
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=f"{binaries}:{os.environ['PATH']}")
            return subprocess.run(["bash", str(scripts / "check.sh"), str(deps)],
                                  env=env, capture_output=True, text=True, check=False)

    def test_v1_with_stale_v2_checkout_uses_v1(self):
        result = self.run_checker('# snip36: low-memory prover\n'
                                  '[patch."https://github.com/starkware-libs/proving-utils"]\n')
        self.assertEqual(result.returncode, 73)
        self.assertIn("V1-checker", result.stdout)
        self.assertNotIn("V2-checker", result.stdout)

    def test_only_exact_active_v2_header_dispatches_v2(self):
        result = self.run_checker('# snip36: low-memory prover\n'
                                  '[patch."https://github.com/starkware-libs/proving"]\n')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "V2-checker\n")

    def test_commented_v2_header_does_not_select_v2(self):
        result = self.run_checker('# snip36: low-memory prover\n'
                                  '# [patch."https://github.com/starkware-libs/proving"]\n'
                                  '[patch."https://github.com/starkware-libs/proving-utils"]\n')
        self.assertEqual(result.returncode, 73)

    def test_missing_active_v2_checkout_fails(self):
        result = self.run_checker('[patch."https://github.com/starkware-libs/proving"]\n', False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("active V2 patch has no proving-v2 checkout", result.stderr)
        self.assertNotIn("V1-checker", result.stdout)

    def test_stock_manifest_does_not_validate_stale_v2(self):
        result = self.run_checker('[workspace]\n')
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("V2-checker", result.stdout)


if __name__ == "__main__":
    unittest.main()
