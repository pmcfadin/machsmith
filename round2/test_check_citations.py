#!/usr/bin/env python3
"""Tests for check_citations.py. Run: python3 round2/test_check_citations.py"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check_citations.py")

LEDGER = [
    {"id": "R2-000001", "kind": "confirm", "T": 0.0832409, "verdicts": {"targets": {"interval": {"point": 0.44187, "lo": 0.43712, "hi": 0.44580}}}},
    {"id": "R2-000002", "kind": "tests", "tests_passed": 10},
]

TABLE = """# w

Cites R2-000002 in prose.

<!-- verdict-table -->

| rule | ratio | interval | T | record |
|---|---|---|---|---|
| a | %s | 0.4371 to 0.4458 | 0.0832 | R2-000001 |
"""


class CheckCitations(unittest.TestCase):
    def run_tool(self, writeup, rule="See R2-000002.\n"):
        with tempfile.TemporaryDirectory() as d:
            ledger = os.path.join(d, "ledger.jsonl")
            with open(ledger, "w") as f:
                for rec in LEDGER:
                    f.write(json.dumps(rec) + "\n")
            w = os.path.join(d, "w.md")
            with open(w, "w") as f:
                f.write(writeup)
            rules = os.path.join(d, "rules", "proposals")
            os.makedirs(rules)
            with open(os.path.join(rules, "r.md"), "w") as f:
                f.write(rule)
            p = subprocess.run([sys.executable, TOOL, "--ledger", ledger, "--writeup", w, "--rules", os.path.join(d, "rules")],
                               capture_output=True, text=True)
            return p.returncode, p.stdout

    def test_ok(self):
        self.assertEqual(self.run_tool(TABLE % "0.4419"), (0, "OK\n"))

    def test_wrong_ratio(self):
        code, out = self.run_tool(TABLE % "0.4420")
        self.assertEqual(code, 1)
        self.assertIn("0.4420 is not in R2-000001", out)

    def test_unknown_id_in_writeup(self):
        code, out = self.run_tool(TABLE % "0.4419" + "\nAlso R2-000099.\n")
        self.assertEqual(code, 1)
        self.assertIn("R2-000099 is not in the ledger", out)

    def test_unknown_id_in_rule(self):
        code, out = self.run_tool(TABLE % "0.4419", rule="R2-000098\n")
        self.assertEqual(code, 1)
        self.assertIn("R2-000098", out)

    def test_row_without_id(self):
        code, out = self.run_tool((TABLE % "0.4419").replace("R2-000001 |", "none |"))
        self.assertEqual(code, 1)
        self.assertIn("cites no ledger id", out)

    def test_no_verdict_table(self):
        code, out = self.run_tool("# w\n\nR2-000001\n")
        self.assertEqual(code, 1)
        self.assertIn("no verdict table", out)


if __name__ == "__main__":
    unittest.main()
