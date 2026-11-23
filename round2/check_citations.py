#!/usr/bin/env python3
"""Traceability check for the round 2 write-up and rule entries (Task 12 Step 4).

usage: check_citations.py [--ledger L] [--writeup W] [--rules DIR]

1. Every `R2-NNNNNN` in the write-up and in every `*.md` under the rules directory must be the id of
   a ledger record.
2. In the write-up, a table that follows a `<!-- verdict-table -->` line is a verdict table. Each of
   its data rows must cite at least one ledger id, and every number in the row printed with three or
   more decimals must equal, rounded to the printed precision, some number in one of the records the
   row cites.

Prints `OK` and exits 0, or prints each problem and exits 1. Standard library only.
"""
import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ID_RX = re.compile(r"R2-\d{6}")
NUM_RX = re.compile(r"(?<![\w.-])\d+\.\d{3,}(?![\w]|\.\d)")
MARKER = "<!-- verdict-table -->"


def load_ledger(path):
    records = {}
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            rec = json.loads(line)
            rid = rec.get("id")
            if rid in records:
                raise SystemExit(f"{path}:{n}: duplicate id {rid}")
            records[rid] = rec
    return records


def floats_in(obj, out):
    if isinstance(obj, bool):
        return
    if isinstance(obj, (int, float)):
        out.append(float(obj))
    elif isinstance(obj, dict):
        for v in obj.values():
            floats_in(v, out)
    elif isinstance(obj, list):
        for v in obj:
            floats_in(v, out)


def matches(printed, values):
    decimals = len(printed.split(".")[1])
    return any(f"{v:.{decimals}f}" == printed for v in values)


def is_separator(row):
    return bool(re.fullmatch(r"[\s|:-]+", row))


def check_ids(path, text, records, problems):
    for n, line in enumerate(text.splitlines(), 1):
        for rid in ID_RX.findall(line):
            if rid not in records:
                problems.append(f"{path}:{n}: {rid} is not in the ledger")


def check_verdict_tables(path, text, records, problems):
    lines = text.splitlines()
    tables = 0
    i = 0
    while i < len(lines):
        if lines[i].strip() != MARKER:
            i += 1
            continue
        i += 1
        while i < len(lines) and not lines[i].strip():
            i += 1
        start = i
        while i < len(lines) and lines[i].lstrip().startswith("|"):
            i += 1
        rows = [(n + 1, lines[n]) for n in range(start, i)]
        if len(rows) < 3 or not is_separator(rows[1][1]):
            problems.append(f"{path}:{start + 1}: no table after {MARKER}")
            continue
        tables += 1
        for n, row in rows[2:]:
            ids = [r for r in ID_RX.findall(row) if r in records]
            if not ids:
                problems.append(f"{path}:{n}: verdict-table row cites no ledger id")
                continue
            values = []
            for rid in ids:
                floats_in(records[rid], values)
            for printed in NUM_RX.findall(row):
                if not matches(printed, values):
                    problems.append(f"{path}:{n}: {printed} is not in {', '.join(ids)}")
    if tables == 0:
        problems.append(f"{path}: no verdict table ({MARKER}) found")


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ledger", default=os.path.join(HERE, "results", "ledger.jsonl"))
    ap.add_argument("--writeup", default=os.path.join(ROOT, "docs", "results", "2026-11-round-2-cqlite-hotspots.md"))
    ap.add_argument("--rules", default=os.path.join(ROOT, "docs", "rules"))
    args = ap.parse_args(argv[1:])

    records = load_ledger(args.ledger)
    problems = []

    with open(args.writeup, encoding="utf-8") as f:
        text = f.read()
    check_ids(args.writeup, text, records, problems)
    check_verdict_tables(args.writeup, text, records, problems)

    for dirpath, _dirs, files in sorted(os.walk(args.rules)):
        for name in sorted(files):
            if name.endswith(".md"):
                p = os.path.join(dirpath, name)
                with open(p, encoding="utf-8") as f:
                    check_ids(p, f.read(), records, problems)

    if problems:
        for p in problems:
            print(p)
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
