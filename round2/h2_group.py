#!/usr/bin/env python3
"""H2 group share by physical function (DECISIONS R22).

usage: h2_group.py <bench binary> <tag>=<profile dir> [<tag>=<profile dir> ...]

A sample belongs to H2 when its leaf physical symbol (perf's symbol, before inline resolution) is in
the bench binary and demangles to one of R22's six functions. The denominator is every flat sample
attribute.py counts (kernel-leaf samples dropped). Prints JSON: per tag the workload share, the share
of each member, the matched symbols, and the per-case share.
"""
import collections
import glob
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import attribute  # noqa: E402

# R22, verbatim order. Each pattern is matched against the demangled physical symbol.
MEMBERS = [
    ("CqlType::parse_with_depth", re.compile(r"CqlType>?::parse_with_depth$")),
    ("SSTableReader::decode_value_for_bench", re.compile(r"SSTableReader>?::decode_value_for_bench$")),
    ("str::to_lowercase", re.compile(r"^(<str>|alloc::str::<impl str>)::to_lowercase$")),
    ("vector_type::split_leading_identifier", re.compile(r"vector_type::split_leading_identifier$")),
    ("ComparatorType::from_cql_type", re.compile(r"ComparatorType>?::from_cql_type$")),
    ("str::trim_matches", re.compile(r"^(<str>|core::str::<impl str>)::trim_matches(::<.*>)?$")),
]


def member_of(demangled):
    for name, rx in MEMBERS:
        if rx.search(demangled):
            return name
    return None


def count_dir(dirpath, binary):
    real = os.path.realpath(binary)
    scripts = sorted(glob.glob(os.path.join(dirpath, "*.script")))
    slugs = [os.path.splitext(os.path.basename(s))[0] for s in scripts]
    cases = attribute.read_cases(dirpath, slugs)
    per_case = {}
    for script, slug in zip(scripts, slugs):
        syms = collections.Counter()
        n = 0
        for _ip, sym, _off, dso in attribute.parse_script(script):
            n += 1
            if dso.startswith("/") and os.path.realpath(dso) == real:
                syms[attribute.strip_hash(sym)] += 1
        per_case[cases[slug]] = (n, syms)
    return per_case


def report(per_case):
    allsyms = set()
    for _n, syms in per_case.values():
        allsyms.update(syms)
    dem = attribute.demangle(sorted(allsyms))
    owner = {s: member_of(dem.get(s, s)) for s in allsyms}
    total = sum(n for n, _ in per_case.values())
    members = collections.Counter()
    matched = collections.defaultdict(set)
    cases = {}
    for case, (n, syms) in per_case.items():
        hit = 0
        for s, c in syms.items():
            m = owner[s]
            if m:
                hit += c
                members[m] += c
                matched[m].add(dem.get(s, s))
        cases[case] = 100.0 * hit / n if n else 0.0
    return {
        "samples": total,
        "h2_share": 100.0 * sum(members.values()) / total if total else 0.0,
        "members": {name: 100.0 * members[name] / total if total else 0.0 for name, _ in MEMBERS},
        "matched_symbols": {name: sorted(matched[name]) for name, _ in MEMBERS},
        "per_case": cases,
    }


def main(argv):
    if len(argv) < 3:
        sys.exit(__doc__)
    binary = argv[1]
    out = {}
    for spec in argv[2:]:
        tag, _, d = spec.partition("=")
        out[tag] = report(count_dir(d, binary))
    json.dump(out, sys.stdout, indent=1)
    print()


if __name__ == "__main__":
    main(sys.argv)
