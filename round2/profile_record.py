#!/usr/bin/env python3
"""Build the body of a `kind: profile` ledger record for one profile tag.

Usage: profile_record.py <tag> <out.json> [workload ...]

Reads work/r2-profile/<tag>/<workload>/host.json and the attribution file
round2/results/profile/<tag>/<workload>.json. Default workloads: every
subdirectory of work/r2-profile/<tag>/ that has a host.json. Exits non-zero if
a profiled binary's sha256 differs from round2/results/profile/binaries.json.

For a branch-stack tag (host.json has "branch": true) the attribution file is
the callers JSON (callers.py): the record carries its samples, bucket_samples
and brstack_empty_pct, and no case_samples or chain stats.
"""
import hashlib
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROFILE_DIR = os.path.join(ROOT, "work", "r2-profile")
RESULTS_DIR = os.path.join(ROOT, "round2", "results", "profile")
BINARIES = os.path.join(RESULTS_DIR, "binaries.json")

NOTES = {
    "A": "flat profile A (spec section 5)",
    "B": "flat profile B (spec section 5); decode and partition_lookup rerun on A's cores (DECISIONS R17)",
    "A-cg": "DWARF call-graph run, structural evidence only",
    "C": "third decode profile for H2 (DECISIONS R22)",
    "A-br": "read branch-stack profile for H3/H5, structural only (DECISIONS R23)",
}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def rel(path):
    path = os.path.abspath(os.path.join(ROOT, path))
    return os.path.relpath(path, ROOT)


def workload_entry(tag, workload, binaries):
    wdir = os.path.join(PROFILE_DIR, tag, workload)
    with open(os.path.join(wdir, "host.json")) as f:
        host = json.load(f)
    bin_path = rel(host["bin"])
    actual = sha256(os.path.join(ROOT, bin_path))
    expected = binaries[workload]["sha256"]
    if actual != expected:
        raise SystemExit(
            f"profile_record: {bin_path} sha256 {actual} != binaries.json {expected}"
        )
    results = os.path.join(RESULTS_DIR, tag, f"{workload}.json")
    with open(results) as f:
        attr = json.load(f)
    entry = {
        "dir": rel(wdir),
        "bin": bin_path,
        "sha256": actual,
        "host": host,
        "results": rel(results),
        "samples": attr["samples"],
    }
    if host.get("branch"):
        entry["bucket_samples"] = attr["bucket_samples"]
        entry["brstack_empty_pct"] = attr["brstack_empty_pct"]
    else:
        entry["case_samples"] = attr["case_samples"]
        if attr.get("callgraph"):
            for k in ("chain_reaches_root", "median_chain_depth"):
                entry[k] = attr[k]
    return entry


def main(argv):
    if len(argv) < 3:
        raise SystemExit(__doc__)
    tag, out = argv[1], argv[2]
    workloads = argv[3:]
    if not workloads:
        tdir = os.path.join(PROFILE_DIR, tag)
        workloads = sorted(
            w for w in os.listdir(tdir)
            if os.path.isfile(os.path.join(tdir, w, "host.json"))
        )
    if not workloads:
        raise SystemExit(f"profile_record: no workloads with host.json under {tag}")
    with open(BINARIES) as f:
        binaries = json.load(f)
    body = {
        "tag": tag,
        "note": NOTES.get(tag, ""),
        "cqlite_commit": binaries[workloads[0]]["cqlite_commit"],
        "workloads": {w: workload_entry(tag, w, binaries) for w in workloads},
    }
    with open(out, "w") as f:
        json.dump(body, f, indent=2, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main(sys.argv)
