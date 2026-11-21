import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import attribute  # noqa: E402


def stripped_env(**keep):
    env = {k: v for k, v in os.environ.items() if k not in ("MSX_LLVM_BIN", "R2_PERF")}
    env.update(keep)
    return env


class RequireEnvTest(unittest.TestCase):
    def run_with(self, **env):
        old = {k: os.environ.get(k) for k in ("MSX_LLVM_BIN", "R2_PERF")}
        try:
            for k in old:
                os.environ.pop(k, None)
            os.environ.update(env)
            return attribute.require_env()
        finally:
            for k, v in old.items():
                os.environ.pop(k, None)
                if v is not None:
                    os.environ[k] = v

    def test_both_set(self):
        self.assertIsNone(self.run_with(MSX_LLVM_BIN="/x", R2_PERF="/y"))

    def test_llvm_missing(self):
        msg = self.run_with(R2_PERF="/y")
        self.assertIn("MSX_LLVM_BIN", msg)
        self.assertNotIn("R2_PERF", msg)
        self.assertIn("source round2/env.sh", msg)

    def test_perf_missing(self):
        msg = self.run_with(MSX_LLVM_BIN="/x")
        self.assertIn("R2_PERF", msg)
        self.assertNotIn("MSX_LLVM_BIN", msg)

    def test_both_missing(self):
        msg = self.run_with()
        self.assertIn("MSX_LLVM_BIN", msg)
        self.assertIn("R2_PERF", msg)

    def test_empty_counts_as_missing(self):
        msg = self.run_with(MSX_LLVM_BIN="", R2_PERF="/y")
        self.assertIn("MSX_LLVM_BIN", msg)


class CliTest(unittest.TestCase):
    def check(self, script, args):
        p = subprocess.run([sys.executable, os.path.join(HERE, script)] + args,
                           env=stripped_env(), capture_output=True, text=True)
        self.assertEqual(p.returncode, 2, p.stderr)
        self.assertIn("source round2/env.sh", p.stderr)
        self.assertIn("MSX_LLVM_BIN", p.stderr)
        self.assertIn("R2_PERF", p.stderr)
        self.assertNotIn("Traceback", p.stderr)
        self.assertEqual(len(p.stderr.strip().splitlines()), 1)

    def test_attribute(self):
        self.check("attribute.py", ["/nonexistent", "/nonexistent", "/nonexistent"])

    def test_callers(self):
        self.check("callers.py", ["/nonexistent", "/nonexistent", "/nonexistent"])

    def test_profile_record(self):
        self.check("profile_record.py", ["tag", "/nonexistent"])

    def test_no_args(self):
        for s in ("attribute.py", "callers.py", "profile_record.py"):
            self.check(s, [])


if __name__ == "__main__":
    unittest.main()
